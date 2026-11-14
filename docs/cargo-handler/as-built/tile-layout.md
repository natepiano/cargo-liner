# Tile trees, sizing, and redraws

## What it is

cargo-handler draws hierarchy groups and one unassigned pool per machine through `tui_pane`.
Shared measurements align neighboring trees and preserve useful columns at narrow widths.
The grid chooses one checked layout and retains it while its relevant inputs stay equal.

Paths below are relative to `crates/cargo-handler/src/` unless they start with `crates/`.

| File | Responsibility |
| --- | --- |
| `tiles.rs` | `TileKey` and `AgentCell` identities. |
| `agent_cell.rs` | Tree lines, grouping, population, measurement, fitting, and demand. |
| `render/mod.rs` | Frame preparation, grid demand, selection areas, and styled titles. |
| `summary/mod.rs` | Machine groups, root membership, and summary layout. |
| `summary/age.rs` | Shared age text and `AgeLayout`. |
| `terminal.rs` | Display deadlines and body-frame decisions. |
| `crates/tui_pane/src/tiles/grid.rs` | Column admission, layout selection, motion endpoints, and painted state. |
| `crates/tui_pane/src/tiles/draw.rs` | Clipped painting, focus fill, and readouts. |

## How it works

### Groups and tree rows

`TileKey` has three kinds: `Agent(AgentCell)`, `Producers`, and `Unassigned { machine }`.
`group_tiles` groups roots after hue assignment.
A root that staff names a producer belongs in the one producers tile.
That tile holds the producers of every machine, each as a flat root with its work below it.
A producer row adds its machine to its name when the tile holds more than one machine.
Any other root with managed members or drawable work belongs in its hierarchy tile and the summary's session table.
A root without hierarchy belongs in its machine's unassigned tile.
It does not also appear as a summary session row.
`TilePool` names the two pools, and `PooledEntry` draws both with the shared tree table.
An empty pool has no tile.
The producers tile comes before the first unassigned tile.
Pool identity survives individual sessions entering and leaving.
The titles are `producers` and `unassigned · <machine>`, in the theme title ink.

`RootPresentation` supplies the summary membership decision, and producers do not change it.
Machine heading counts follow the sessions that the summary lists.
Count text uses `top-level agent` or `top-level agents`.
The unassigned count follows the current pooled rows.

`cell_order_placed` constructs agent entries and tree lines from `AgentPlacement`.
Root headings show the reported status in the same ink as the tree status cell.
An unavailable status has no heading segment or separator.
Headings omit age and detailed state words.
Expansion actions and redraw deadlines use the same account status source as the frame.
`ChildLine` classifies work, carriers, and separate sessions exhaustively.
Shells, subagents, and worker threads draw once as work lines.
Carrier processes contribute kept descendants without drawing another line.
A managed session or thread leaves its former owner's work list.
Cross-machine descendants retain their machine label.
Pool roots use flat branches; their descendants keep ordinary tree guides.

`TreeColumn` draws Session, Account, Role, Activity, Status, Branch, Type, Model, Effort, and Directory.
Age remains a sort key and appears in the details view.
Type identifies shells and subagents; session and worker rows leave it blank.
Role shows unit, showrunner, producer, or worker.
Activity shows the assigned work or `timer <countdown>`.
An expired timer shows `timer` and keeps status `busy`.
A subagent shows role `worker` and its source role word in Activity.
Model and effort come from the conversation's own census fields.
Unknown values and shell metadata remain blank.
See [activity](activity.md) for status and role details.

`AccountColumn::of` makes one frame-wide decision across all rows and machines.
Two labels for one program make accounts visible everywhere.
Unrecorded accounts do not count as a second label.
Header program labels and summary labels use that decision.
Tree account columns retain recorded accounts.

### Measurement and column fit

Frame preparation restores tree viewports before source measurement.
`TreeMeasurement` measures each tile for the frame.
Its tallest projection supplies the minimum width before grid layout.
The minimum reserves tree guides, a name cell, role space, the status header, gaps, indents, and frame.
It does not reserve the widest changing status value.
This prevents activity text from changing the number of tile columns.

`TreeColumnPopulation` counts values per tile.
`PermittedTreeColumns` retains columns with values.
Role, activity, status, and session retain their headers before width fitting.
`shared_tree_metrics` groups tiles by screen x and permitted columns.
`TreeMetrics` shares widths and `AgeLayout` among matching groups.
A final pass fits the union of column sets across all tiles at the same x.
Each tile at that x uses the same session width, column slots, and gaps.
A tile without a column reserves its slot but draws no header or values there.
A tile alone at its x keeps its local layout.
`PresentationValues` measures role and status widths for each presentation.
Those widths follow rows that the allocation paints.
Population and ages still consider complete rows.

`GroupFitStep` and `ColumnDrop` fit columns in this order:

1. Remove effort.
2. Remove model.
3. Remove directory.
4. Remove type.
5. Remove branch.
6. Cut session names to the remaining width.
7. Remove account only below the session minimum.
8. Remove role.
9. Remove activity.
10. Remove status.

Each step acts only when the current columns exceed the available width.
The session minimum holds the session header and the deepest tree guides.
Matching fitted groups merge after each step.
Trees in one screen column share the fitted session width.
`FittedTreeColumns` supplies headers, values, and painted header spans together.
`fit_tree_names` retains the session minimum before it reduces the final gaps.
Tree guides remain intact before the shortened name.
The details view retains the full name and the values of removed columns.

`summary::truncated_line` measures terminal cells and preserves grapheme boundaries and span styles.
Cut text ends with `…`.
Numbers and status words remain whole.
Unused width goes to directory when present.
The column union uses the smallest tile width at the same x.
A summary session table with no drawable columns uses `SummarySessionTable::OmittedForWidth` and requests no rows.

`CellView::requested_at_width` determines row demand at the candidate width.
`TreePresentation::HeightCompressed` applies only after row allocation.
Height compression must not feed a smaller requested demand into the next layout decision.
Motion preparation measures at the taller source or destination height.
Selection areas are installed only after shared metrics resolve.

### Grid admission and stable layout

cargo-handler selects `TileColumnOrder::LongestWithShortest` through `TileCells::column_order`.
The shared grid keeps source order by default, including cargo-tile.
`Cells` records each full tree count before expansion removes hidden rows.
The count includes the root and every descendant.
It excludes headings, wrapped text, and the tile readout.
Collapse, scrolling, and viewport height do not change these counts.

The grid ranks detail tiles by full tree count, longest first.
Equal counts retain their previous ranking.
The first ranking uses source order for equal counts.
It assigns ranks across columns in snake order: left to right, then right to left.
A full column receives no further rank in that pass.
Each column retains source order among its assigned tiles.
The summary and empty slots keep their positions.
The placement retains the selected column capacities, summary depth, and summary span.
With four detail slots in two columns, counts `42, 42, 11, 3` produce pairs `42, 3` and `42, 11`.

`ColumnRanks` stores identities in rank order rather than changing row counts.
A count change that preserves the ranking and grid geometry does not change assignment.
Grid geometry includes pane size, column count, and summary depth.
A summary depth change assigns the same ranks over the new column capacities.
A rank, membership, or column-count change invalidates the selected placement.
The existing grid motion animates each reassignment.
Frame preparation recomputes the column union after assignment.
Measured widths remain separate from fitted drawing widths.
The next painted frame records the new header positions for clicks.

cargo-handler selects `TileWidthFloor::CompressSingleColumn`.
It retains one compressed tile column below the core width.
Generic grids with `Required` can show the summary alone when no width fits.
`SummaryRowAllocation::BeforeTiles` reserves missing summary rows before assigning the remaining tile rows.

`ColumnLayout::checked` constructs `RowAdmissibleColumnLayout`.
N framed residents require `N × (minimum_height − 1) + 1` rows because adjacent frames share a border.
Summary redistribution also checks the resident band below a widened summary.
A width policy never lowers the height floor.
Excess tiles remain demanded without receiving an invalid allocation.

`TileGrid::sync_measured_demands` searches permitted column counts at their own candidate widths.
`LayoutSelectionInputs` includes area, arrangement, growth, endpoint rules, and demand measurements.
`LayoutSelection` retains selected widths, demands, and depth.
Equal inputs avoid another candidate search.
An unchanged draw makes two provider demand calls.
Source-order preparation runs once before those calls.
Ranked placement prepares the new column memberships before painting.

`LayoutDemandMeasurement` equality compares widths and requested rows, while ignoring preferred summary width alone.
A new summary width remeasures cached candidates before reuse.
`CandidateMeasurementFreshness` records whether only current widths or every cached width is current.
A row change through `RowDemandChange` updates held rows and queues the resize from painted slots.
Added tiles, removed tiles, and terminal size changes can select another layout.

`update_layout_rules` owns area and endpoint-rule changes.
A changed rule replans endpoints, clears obsolete queued motion, and settles the old motion.
`EndpointLayoutRules` travels with each endpoint.
`CheckedLayoutAdmission` avoids repeated checks for unchanged endpoints.
`complete_arrangement` installs whole slot sets and reconciles focus.
For a widened summary, `GridLines::open` removes internal band dividers after every band frame has been added.
A later frame insertion would restore a divider through summary text.
`ClippedPaneDestination` bounds moving content and scratch-buffer blits to the visible piece.
A divider clamp cannot repair invalid admission.

### Ages, readouts, and repaint

`age_label` draws whole elapsed minutes as `d:hh:mm`, omitting fields that do not exist.
Examples are `0`, `10`, `1:10`, and `1:02:10`.
`AgeLayout` measures the widest label, including offscreen rows.
Age headers and values align right.
Accounts reading freshness keeps its separate format.
Each conversation changes age at its own elapsed-minute boundary.

`RowsReadout::{Count, Selected}` supplies the tile foot.
`RowsReadout::selected` checks that the selected position does not exceed the total.
A selected surface shows `content rows: N of M`.
An unselected surface shows the content count.
The fit ink uses measured content rows, rather than the selectable total.
Wrapped accounts can therefore need more display rows than their selection count.
The foot remains chrome and supplies no word target.

`LastPainted` on `TileGrid` retains visual state, theme, transparency, tint, and summary frames.
`visual_repaint` compares the next visible drawing with that state without consuming the result.
Only a body paint replaces the record.
The host also checks content changes before omitting a body paint.
`DisplayText` supplies deadlines for agent ages, shared age-width growth, shell countdowns, and account freshness.
The status clock can reuse the retained body between those changes.

## Invariants and gotchas

1. Preserve one membership decision for summary sessions and unassigned roots.
2. Measure guides and names through the same fit used for drawing.
3. Keep per-tile population when merging column metrics.
4. Validate rows and width before admitting residents.
5. Replan cached endpoints after area or rule changes.
6. Resolve input from painted geometry during motion.

A change visible only at an unmeasured candidate width can remain unseen until another measured input changes.
A current width list absent from the cache forces another search.
A layout rule assigned outside `update_layout_rules` can leave stale endpoints.
A slot install outside `complete_arrangement` can leave focus on a removed group.
Long trees scroll or clip within their tile; they do not continue into another column.
See [interaction](interaction.md) for selection, scroll, and painted input.

## Why

One measurement contract keeps demand, drawing, headers, and input consistent.
Checked admission prevents impossible divider budgets.
Retained layout inputs prevent summary text changes from repeatedly rearranging tiles.
Visual and text deadlines reduce idle drawing without hiding changing content.
