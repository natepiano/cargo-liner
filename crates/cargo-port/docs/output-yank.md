# Output-pane yank — decisions

2026-05-30 — decisions from investigation of partial text selection/yank in the output pane.

1. Reuse existing `y`/copy plumbing → **approved**. Implement `CopySelection<App>` for OutputPane and register it; `y` yanks the selection through the existing framework clipboard path. No new key or clipboard code.

2. Scroll freeze-vs-follow → **approved**. Follow the tail while output streams; freeze the view on scroll/select. Render must read `viewport.scroll_offset` instead of pinning to bottom. Prerequisite for selection; also fixes the currently-dead wheel scroll.

3. Selection UX → **Option D** (linewise keyboard core + wheel scroll + line-granular mouse drag). Whole-line selection only; character/sub-line spans deferred.
   - Constraint: the output pane must become a standard `Navigable` pane reusing the app's existing `NavigationAction` set, **not** a bespoke key set. The bindings already allowed — `PageUp`/`PageDown`, `HalfPageUp`/`HalfPageDown`, and `j`/`k` in vim mode — must move the cursor and, while a `V` selection is active, extend it. So `V` + any of those motions extends the linewise selection.

4. Phasing → **keyboard first, mouse second** (user deferred to recommendation). Phase 1: navigable cursor + `V` linewise selection (with page/half-page motions) + `y` yank + scroll freeze/follow. Phase 2: wheel scroll + drag-to-select-lines. Character-level selection deferred.
