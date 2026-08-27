# cargo-tile favorites — next items

Work that came out of the favorites plan but does not belong to any of its phases.
An item here is a candidate, not a commitment: nothing is scheduled until it is
written into a phase.

## Toast entrance frames are requested before the toast can change height

`ToastVisualTimeline`'s entrance leg asks the event loop for 8ms frames from
`pushed_at`, but `tui_pane`'s `current_visible_lines` clamps the rendered height
up to `min_height`, so nothing on screen changes until
`pushed_at + (min_height - 1) * entrance_line_ms`. Every one of those frames
redraws an unchanged toast. For an ordinary single-line toast — the common case,
where `target_height == min_height == 3` — the entire entrance leg is redundant.

Give the timeline an entrance **start** as well as an end, at
`pushed_at + (min_height - 1) * entrance_line_ms`. Do not special-case
`target_height == min_height`; that would leave multi-line toasts wasting the same
leading window. Preserve multi-line and exit-boundary behavior, and add a
single-line regression proving no entrance repaint is requested before expiry.

Correctness is unaffected — this is frame economy on a loop the project has
already tuned for idle cost, which is why it is a backlog item rather than
remaining feature scope.

## Two attract states are hidden behind bare options

`Attract` keeps `identified: Option<bool>`, which collapses three real states
into two: the window has not been looked for yet, the search ran and settled on
nothing, and the search found the window. A reader cannot tell `None` meaning
"not observed" from `None` meaning "observed and unsettled" without following
every writer.

`Attract::keyed_mode() -> Option<AttractMode>` has the same problem on the input
path: `None` means the keystroke passes through to the app rather than "no mode
exists", and only the call site tells the two apart.

Replace both with named enums — an identification state carrying the
not-observed / unsettled / settled distinction, and a key-routing type carrying
pass-through versus a chosen mode. Neither is a behavior change, both are
mechanical once the enums exist, and the compiler finds every site.

## The favorites overlay file holds four independent type clusters

`crates/cargo-tile/src/favorites_overlay.rs` is about 1,800 lines of non-test
code, and the module-splitting rule asks for a split when two of its four tests
hold. Three hold here. The file defines well over a dozen top-level types that
never appear in each other's field lists; it mixes four domains — the displayed
content (`FavoritesOverlayContent`, `FavoriteRowsView`, `FavoriteRowView`,
`UnrecognizedFavoritesView`), keymap binding resolution
(`FavoritesSurfaceBindings`, `ModeColumnBindings`, `ParameterColumnDescriptor`),
the width and line cache (`CachedOverlayLine`, `CachedLinePlan`,
`CachedSurfaceWidth`, `FavoriteSectionTableLayout`), and the overlay's own state
machine (`FavoritesOverlay`, `FavoritesOverlayNotice`,
`FavoriteRemovalCommitState`, the outcome enums); and each of those four would
carry a focused test module instead of the single 1,250-line one the file has
now.

Split it into `favorites_overlay/mod.rs` with a submodule per cluster, named
after each cluster's anchor type, and move each cluster's tests down with it.
This is a structural change with no behavior in it, which is why it is a backlog
item rather than a defect: the file works, it is just too large to navigate.

## The favorites footer advertises actions that may not apply

`FavoritesSurfaceBindings::footer` in `crates/cargo-tile/src/favorites_overlay.rs`
builds its hint line unconditionally, so the overlay offers load and delete even
when the table has no row under the cursor — an empty favorites file, or the
cursor parked in the unrecognized-entries block below the table. Pressing the
key is harmless, but the footer is describing an action the overlay will refuse.

Make the footer read the current selection: drop the load and delete hints when
`favorite_selection()` returns `Nothing`, so the line only ever names keys that
will do something.

## A production helper is named after its test

`parse_rows_for_overlay_test` in `crates/cargo-tile/src/favorites.rs` is
production code — the overlay calls it to turn a favorites file into rows — but
its name says it exists for a test. A reader looking for the parse the overlay
actually uses skips straight past it, and a reader cleaning up test scaffolding
might delete it.

Rename it for what it does rather than who first called it: it parses the rows
the overlay displays, so `parse_rows_for_overlay` states the whole of it.

## Two different `mode_label` helpers share one name

`favorites_overlay.rs` and `globals.rs` each define a private `mode_label`, and
they disagree: the overlay returns title case ("Moving Band") because it heads a
table column, the toast returns sentence case ("Moving band") because it sits
mid-sentence. Both are correct where they are, but the shared name reads as one
helper used twice, so a future edit to "the" `mode_label` will silently change
casing in the surface it was not looking at.

Give each the name of its own surface — `column_mode_label` and
`toast_mode_label`, or equivalent — so the casing difference is visible at the
call site instead of only in the body.
