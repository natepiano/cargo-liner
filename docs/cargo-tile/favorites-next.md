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
exists", and only the caller's shape reveals which.

Replace both with named enums — an identification state carrying the
not-observed / unsettled / settled distinction, and a key-routing type carrying
pass-through versus a chosen mode. Neither is a behavior change, both are
mechanical once the enums exist, and the compiler finds every site.
