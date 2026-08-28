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

## The moving band sits on flat color instead of on the desktop

The moving band draws its characters over the theme's background color, so the
band reads as a rectangle pasted onto the window rather than as something moving
across what is behind it. The screenshot shows the effect plainly: the strip is a
hard-edged column of blue characters, and everything it has passed over is one
flat tone.

Two changes together fix it. Sample the desktop behind the window and use that
as the band's background, the way `attract_pixelate` already does — the capture
machinery exists and is enabled for the attract screen, so this is a matter of
routing it to `moving_band.rs` rather than building it. Then fade the characters
toward that background as they approach the band's leading and trailing edges,
instead of ending the band on a hard cut, so the strip reads as passing over the
desktop rather than being laid on top of it.

Worth checking whether `moving_text.rs` wants the same treatment; it has the same
flat-background problem wherever its characters stop.

## A second terminal window loses the desktop capture and gets told it is a permissions problem

Running `cargo tile` in two windows of the same iTerm2 — an app that already has
Screen Recording permission — leaves the second one with no desktop capture. The
attract screen there draws nothing, and after the grace period the status line
says `attract: no desktop capture -- allow Screen Recording for this terminal in
System Settings > Privacy & Security`. The first window keeps its capture.

The message is wrong in this case, and wrong in the most expensive way: it sends
the user to a settings pane to grant a permission that is already granted, where
they will find nothing to change. Whatever the capture is failing on, it is not
the permission.

The reporting itself is sound — `backdrop_overdue` deliberately waits out a grace
period so a slow capture is not mistaken for a missing one, and a screen drawing
nothing says why instead of looking like it never started. What is missing is a
second cause: the notice assumes the only reason a capture never arrives is the
permission, so every other reason inherits that text.

Two things to find out, in this order. First enumerate the windows the window
server actually reports while both instances are running, from outside the
program, and see how each instance resolves its own window among them — a bug
that appears only with two windows of one app is a disambiguation bug, and the
last two backdrop defects were both found this way rather than by reading
branches. Then give the notice a second message for the non-permission case.

The capture lives in `tui_pane`'s `backdrop` feature, not in cargo-tile, so the
fix belongs there — a framework that only works for the first caller to ask is
the framework's defect, not the caller's, and `cargo-port` wants the same
behavior. Fix it where it lives rather than working around it from cargo-tile.

## The unrecognized block gives the arrow keys nothing to land on

Scrolling past the last recognized favorite carries the view down into the
unrecognized block, and from there the arrow keys look like they are moving a
cursor nobody can see: presses are absorbed, the view shifts, and no row ever
takes the highlight that every recognized row takes.

Nothing is actually broken underneath. `FavoritesOverlayContent::saved_count`
counts only recognized rows, so the viewport bounds the selection to those, and
the block's lines are built by `append_unrecognized` as `CachedOverlayLine::Static`
— which `rendered_line` returns untouched, with neither the `▸ ` marker nor
`selection_style`. A static line cannot show selection by construction, and it is
not supposed to, because it is not selectable. The gap is that the overlay never
says so, and the scrolling makes it look like it should be.

Make the broken rows selectable and deletable. A favorite the overlay can show
you but cannot let you remove sends you to a text editor to fix your own config,
which is the one thing the overlay exists to spare you — and delete is the action
that makes sense for an entry nothing can load. That means giving each
unrecognized row its own `CachedOverlayLine::Favorite`-style variant carrying the
row's identity, counting them in the viewport length, and letting the delete key
reach them while load continues to refuse.

If they are instead left unselectable, the block needs to say so — keep the
selected row visible while the view scrolls past it, or mark the block plainly as
a diagnostic rather than a list — so the arrows stop implying a cursor is there.

## The README does not mention favorites at all

`crates/cargo-tile/README.md` documents the attract screen's steering keys in
detail — the arrows, `>` and `<`, `+` and `-`, `v`, `t`, and `1`/`2`/`3` — and
says nothing about the feature built on top of them. The word "favorite" does not
appear in the file. Neither does `ctrl-s`, `ctrl-o`, `m`, `r`, or `u`.

So every key this work added is undiscoverable outside the keymap overlay: saving
the current parameters, opening the favorites table, loading a random favorite
(`RandomFavorite`), randomizing the current settings (`RandomizeAttract`), and
undoing the last replacement (`UndoAttractReplacement`).

Add a favorites section beside the existing attract one, in the same voice:
what a favorite is, where the file lives
(`<os config dir>/cargo-tile/favorites.toml`), the keys that write and read it,
what happens to entries a newer version wrote that this one cannot read, and the
fact that saving the same parameters twice updates the existing row rather than
adding a second.

## The overlay does not show which favorite is the one running

Opening the favorites table while the attract screen is running a set of
parameters that exactly matches a saved favorite gives no sign of it. The table
shows four rows and the cursor sits wherever it was left, so the one favorite
that describes what is on screen right now looks like any other.

Mark it with a `*` at the left of its row.

The row already opens with a two-column marker: `rendered_line` writes `"▸ "`
for the selected row and `"  "` for every other one. Currency is independent of
selection — the running favorite may or may not be the one under the cursor, and
both need to be visible at once — so the two states want their own columns rather
than a combined glyph. Widen the marker to hold selection in the first column and
currency in the second, and compare each row's `settings` against the attract
screen's current settings to decide it, the same equality `FavoriteRows::push`
already uses to recognize a repeat save.

Worth deciding at the same time whether the mark survives an edit: once the user
steers away with an arrow key the settings no longer match, and the mark should
clear rather than going stale.
