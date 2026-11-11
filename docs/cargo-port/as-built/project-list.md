# Project list, columns, and expansion

The project list owns project hierarchy, selection, expansion, and its derived visible rows.
The renderer fits names around stable value columns.

Paths below are relative to `crates/cargo-port/src/`.

## Ownership and lookup

`tui/project_list_state.rs` defines `ProjectList`.
`tui/project_list/list.rs` implements indexed lookup and tree updates.
`ProjectPathLookup::{Found, UnknownPath, StaleRoute}` makes missing and stale routes explicit.
`tui/project_list/visible_rows.rs` derives the navigable rows.

`tui_pane::ExpansionModel<ExpandKey>` owns expansion overrides.
The application default closes every depth.
`tui/project_list/expand_state.rs` supplies application row relationships.
Root replacement retains expansion keys that still identify expandable rows.

`ProjectList::press_row(row, NonRustInclusion) -> RowPressResult` calls the shared model.
It recomputes visibility after `RowPressResult::Toggled`.
`tui/interaction.rs` routes a normal left click through this operation.
`ClickMode::FocusOnly` changes focus without toggling expansion.
There is no application copy of the shared toggle algorithm.

## Columns and names

`tui/columns/project.rs` defines `ProjectListWidths` and `ProjectColumnLayout`.
The layout records preferred, minimum, and already-shown widths for pane allocation.
Names give up width before the value columns disappear.
At narrow widths, Main, Sync, and Disk release unused reserve before the column-drop fallback applies.
Below the supported complete-header width, the fallback drops Main, Sync, Git, Type, CI, and Lint in that order.

`MAIN_COMPARISON_VALUE_RESERVE` reserves five Main cells before comparison data arrives.
Sync has no equivalent fixed reserve.
The divider therefore stays steady for normal loading transitions.

At window widths 60–80, Disk uses compact whole units.
Examples include `52 GiB` instead of a decimal value.
The column renderer preserves one blank cell at each edge.
Headers align with their data columns.
The Projects label starts at the name indent.

With Lint enabled, the Projects header can draw as `Proj…` only when the name column already cuts names.
The three-cell indent and one-cell margin remain.
Other labels do not cut.

`without_shared_prefix` returns a named `MemberName` outcome.
It removes a workspace prefix only at a `-` or `_` boundary when the member needs those cells.
Cut member names therefore retain a useful differing suffix.
Same-name roots use `grouping::differing_path_labels` to expose differing path segments.

Group rows remove their count when it would leave only one name letter.
If necessary, they also remove the tree mark and give those cells to the name.
A remaining cut uses `…` and retains a name letter when the budget permits it.

## Visibility and selection

Deleted rows remain visible until dismissal or rescan.
Only their name and value text receive strike-through.
Disk supplies the dismissal control.
`theme_roles::paint_selection` paints the final row selection across its whole inner width.
Disk does not receive a separate selection fill.

Selection changes record the user's last explicit choice.
A rescan's first-visible-row fallback does not replace that saved choice.
See [watcher behavior](watcher.md) for late replies and deletion transitions.

## Invariants and gotchas

1. Use the shared expansion operation for clicks.
2. Keep value headers and values aligned.
3. Shorten names before dropping supported columns.
4. Keep the Main reserve independent of Sync arrivals.
5. Preserve distinct member and checkout labels after fitting.

A Projects width change also changes Package, Git, Languages, and Targets geometry.
Validate these panes through full-window drawn tests.
See [pane presentation](presentation.md) for shared allocation and focus rules.
