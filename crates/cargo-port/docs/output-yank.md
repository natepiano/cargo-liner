# Output-pane yank — decisions

2026-05-30 — decisions from investigation of partial text selection/yank in the output pane.

1. Reuse existing `y`/copy plumbing → **approved**. Implement `CopySelection<App>` for OutputPane and register it; `y` yanks the selection through the existing framework clipboard path. No new key or clipboard code.

2. Scroll freeze-vs-follow → **approved**. Follow the tail while output streams; freeze the view on scroll/select. Render must read `viewport.scroll_offset` instead of pinning to bottom. Prerequisite for selection; also fixes the currently-dead wheel scroll.

3. Selection UX → **Option D** (linewise keyboard core + wheel scroll + line-granular mouse drag). Whole-line selection only; character/sub-line spans deferred.
   - Constraint: the output pane must become a standard `Navigable` pane reusing the app's existing `NavigationAction` set, **not** a bespoke key set. The bindings already allowed — `PageUp`/`PageDown`, `HalfPageUp`/`HalfPageDown`, and `j`/`k` in vim mode — must move the cursor and, while a `V` selection is active, extend it. So `V` + any of those motions extends the linewise selection.

4. Phasing → **keyboard first, mouse second** (user deferred to recommendation). Phase 1: navigable cursor + `V` linewise selection (with page/half-page motions) + `y` yank + scroll freeze/follow. Phase 2: wheel scroll + drag-to-select-lines. Character-level selection deferred.

---

## Team review (2026-05-30, 1 cycle)

Posture: strengthen. 4 expert lenses (correctness, architecture, risk, ergonomics). No premise-challenge survived the firewall — every "blocker" agents raised is an implementation gap to fix, not proof the approach fails.

### Mechanical / converged (auto-recorded, accepted)

These have one sensible in-intent outcome; recorded, not surfaced.

- **M1 — Render must read scroll state.** `src/tui/panes/output.rs:53` hard-codes the tail offset and never reads `pane.viewport`. Render must use `viewport.scroll_offset()` and populate `set_len(lines.len())`, `set_viewport_rows(inner_height)`, `set_content_area(area)` so cursor/selection math works. (Implements decision 2.)
- **M2 — Wire navigation dispatch.** Change `OutputPane::mode()` from `Mode::Static` to `Mode::Navigable` (`framework_keymap.rs:705`), remove `AppPaneId::Output` from the navigation-dispatch exclusion arm in `panes/actions.rs`, and add a `navigate_output` arm that drives `viewport` up/down/page and extends the selection when active. Without this, `j`/`k`/Page* never reach the pane.
- **M3 — Strip ANSI before clipboard.** `example_output` lines carry raw ANSI; the yank text must be stripped to plain text before `CopyPayload` (hand-rolled strip or a small dep).
- **M4 — Selection state lives on `OutputPane`, not `Viewport`.** Add `selection_anchor: Option<usize>` (+ active flag) to the binary's `OutputPane` struct, matching the precedent that panes hold their own non-cursor state (e.g. ProjectListPane). Keeps the reusable `tui_pane` lib from growing a feature only one pane uses.
- **M5 — Highlight the selection.** Render must style the `[min(anchor,cursor)..=max]` line range (and the cursor line) or the feature is unusable.
- **M6 — Add the `V` action + binding.** New `OutputAction::SelectLinewise` bound to `V`, always available. Register the new actions so they appear in the `?` shortcuts and `Ctrl-k` keymap overlays.
- **M7 — Implement + register `CopySelection<App>` for `OutputPane`.** Return `Nothing` when no selection; else the joined, ANSI-stripped line range. Add `.register_copy_selection::<OutputPane>()` at `framework_keymap.rs:812` (currently missing).
- **M8 — Bounds-clamp the range every frame.** Clamp anchor and cursor to `[0, buffer.len())` before indexing, so a shrinking/streaming buffer can't panic or yield a stale yank.
- **M9 — Yank toast shows line count.** Generic "Copied row" misreads for a multi-line yank; show "Copied N lines" (pane-level message or a new `CopyLabel`).
- **M10 — Accept the Nav region in the status bar.** Making the pane navigable surfaces the Up/Down/Page slots in the bar; that aids discoverability and is consistent — accepted, no special "navigable-without-bar" mode.

### Proposed user decisions (surfaced)

- **D1 — Selection stability while output streams.** `apply_example_progress` overwrites the last line and the buffer grows while a child streams; a frozen selection can drift, yank stale lines, or (without M8) point past the end. **Decided: snapshot the buffer when `V` is pressed** — selection renders and yanks against the frozen copy, so streaming can't drift it. (important)
- **D2 — Freeze/follow affordance + resume.** Decision 2 approved freeze-on-scroll/follow-the-tail but didn't define how the user sees state or returns to following. **Decided: indicator + auto-resume** — show following-vs-frozen state (title or scroll label); `G`/End jumps to the bottom and resumes following; process exit also resumes. (important)
- **D3 — Esc semantics with an active selection.** Esc closes the pane today. **Decided: status-aware** — Esc clears an active selection if one exists, otherwise closes the pane. Single-press close is preserved when not selecting; matches vim. (minor)
- **D4 — Large-selection clipboard guard.** OSC52 silently truncates large payloads (~74–100 KB) in many terminals; a big log yanks incomplete with no error. **Decided: no guard** — accept terminal behavior, keep it simple. (risk accepted)

### Phase-2 note (recorded, not surfaced now)

- Mouse drag-select competes with native terminal selection under mouse capture; behavior varies by terminal (xterm vs tmux vs macOS Terminal). Decide a modifier escape hatch (Shift/Alt-drag) and/or document the limitation when Phase 2 is built.

---

## Team review cycle 2 (2026-05-30)

Posture: strengthen. 4 lenses (correctness lifecycle, state-machine consistency, Rust type-system, testability/ordering). No premise-challenge survived. Cycle 1 produced the task list; cycle 2 mapped the combined state machine and found the undefined transitions between its states. All refinements below are converged (one sensible in-intent outcome) and auto-recorded; two new behavior choices are surfaced.

### Refinements (auto-recorded, accepted)

- **M11 — Model selection + scroll as enums, not parallel Option/bool.** Per house style (make illegal states unrepresentable), replace `selection_anchor: Option<usize>` + active flag with `enum OutputSelection { Inactive, Active { anchor: usize, snapshot: Rc<[String]> } }` (collapses D1's snapshot and M4's anchor into one type — they can't desync), and the D2 follow/frozen flag with `enum ScrollMode { FollowTail, Frozen { offset: usize } }`. Supersedes the M4/D1 "anchor + flag + separate snapshot" sketch.
- **M12 — Cursor/scroll coupling (the model `Viewport` leaves open).** `Viewport.pos` (cursor) and `scroll_offset` are independent. Resolution: navigable cursor; scroll follows the cursor to keep it visible; `ScrollMode::FollowTail` pins to the bottom each frame; any manual cursor move / wheel scroll → `Frozen { offset }`; `G`/End → `FollowTail`. (Consistent with every other navigable pane.)
- **M13 — `V` is a modal selection, not a one-shot.** `V` enters linewise-visual mode and snapshots (D1); while active, every motion (`j`/`k`/Page/half-page/`g`/`G`) extends the range; `y` yanks and exits; `Esc` cancels and exits (matches D3). While not selecting, the same motions just move the cursor.
- **M14 — Post-yank state.** After `y`: clear the selection, discard the snapshot, resume `FollowTail`. Resolves the gap that D3 (Esc) didn't cover.
- **M15 — Render source + clamp against the snapshot (refines M8).** When `OutputSelection::Active`, render and yank read the snapshot, not the live buffer; clamp anchor/cursor to `[0, snapshot.len())` every frame. When `Inactive`, render the live buffer.
- **M16 — Initial state.** Pane opens in `FollowTail` with the cursor at the bottom; empty-buffer and single-line cases are safe (no index panics).
- **M17 — Follow auto-scroll hook.** In `FollowTail`, render computes the tail offset (`len - inner_height`) each frame so new output stays visible; `apply_example_progress` needs no viewport callback.
- **M18 — `CopyLabel` stays generic; format in the binary.** Don't add output-specific jargon to the lib's `CopyLabel`. The binary formats "Copied N lines" (add a generic count-bearing variant only if another pane needs it). Keep raw `usize` indices (newtype would be over-engineering here); snapshot held as `Rc<[String]>` to avoid cloning on every `V`. (Refines M9.)
- **M19 — Acceptance tests (auto-applied; test additions are minor).** Integration tests via the existing `make_app` + injectable `ClipboardBackend` (`FakeClipboard`) harness in `src/tui/app/tests/`: `V` enters selection + snapshots; nav extends range; range clamps on shrink/overrun; `y` yields ANSI-stripped joined payload + "Copied N lines"; `y` with no selection → `Nothing`; status-aware Esc (clear vs close); snapshot stable while `apply_example_progress` mutates; freeze/follow toggle + `G`/End resume + process-exit; `V` action surfaces in `?`/keymap overlays; `CopySelection` registered for `AppPaneId::Output`; Navigable mode renders the Nav region. (Copy-payload generation extracted to a pure helper for unit testing; full path tested via integration.)
- **M20 — Implementation order (build stays green each step).** M2 (Mode::Static→Navigable + remove the `panes/actions.rs` Output dispatch exclusion + `navigate_output`) lands before M1 (render reads viewport). Micro-steps: (1) Navigable + `navigate_output` stub; (2) `OutputAction::SelectLinewise` bound to `V` + dispatch stub; (3) render reads viewport + `OutputSelection` field; (4) ANSI strip + `CopySelection` impl/register; (5) clamp + toast label; (6) freeze/follow + indicator + resume.
- **M21 — Regression check on M2 (no action; reassurance).** Removing `AppPaneId::Output` from the navigation-dispatch exclusion is isolated: `edge_scroll_probe` and `list_cursor` already special-case Output and stay correct; no other pane depends on Output being non-navigable; the only other effect is the Nav region appearing in the bar (M10, accepted).

### Proposed user decisions (surfaced)

- **D5 — Process exit while a selection is active.** D2 says process exit resumes following; D1 freezes the selection on a snapshot. These collide: resuming-follow on exit would discard a selection the user is mid-way through. Options: clear the selection + resume following on exit / an active selection suppresses the auto-resume until it's cleared (then resume). **Decided: suppress the auto-resume while a selection is active** — the selection holds the view; the pane resumes following only after `y` (yank) or Esc (cancel), so a process finishing never destroys an in-progress selection. (important)
- **D6 — `y` with no active selection.** M7 returns `Nothing` ("Nothing to copy" toast). But the global `y` copies the focused row on every other pane, so a user may expect `y` to grab the current cursor line here. Options: `Nothing` (selection required) / yank the current cursor line as a fallback. **Decided: `Nothing`** — copying in the output pane requires a deliberate `V` selection; `y` with no selection shows "Nothing to copy" (confirms M7). (minor)

---

## Phase 1 implemented (2026-05-30)

Keyboard yank shipped. Refinements to the recorded model, all consequences of M12 (cursor-driven scroll):

- **No `ScrollMode` field — follow is derived from the cursor.** M11 sketched `enum ScrollMode { FollowTail, Frozen { offset } }`; the first implementation kept a payload-free `FollowTail | Frozen` field. Both are a second source of truth for something the cursor already encodes: following *is* "the cursor sits on the last row" (`pos >= len - 1`). The field is removed entirely. `is_following()` derives from the viewport; navigation is the shared `navigate_viewport(&mut viewport, action)` every scroll pane uses (so vim `hjkl`, page, half-page, Home/End come for free); the only stateful bit — stick to the tail as new lines append — lives in `sync_viewport`, which re-pins the cursor to the new last row when it was on the old one and no selection holds the view. The `FollowTail`/`Frozen` contradiction (mode disagreeing with the cursor) is now unrepresentable.

> **Superseded by the non-modal redesign (2026-05-31), below.** The model `OutputSelection { Inactive, Active }`, the `SelectLinewise` action, and the two-color highlight described in the original Phase-1 bullets were all replaced. Read the redesign section for the shipped behavior.

Phase 2 (mouse wheel scroll + drag-to-select-lines) and character-level selection remain deferred.

---

## Non-modal redesign (2026-05-31)

The modal "press `V` to start selecting" model was collapsed into an always-present selection: there is no select/deselect mode, the cursor row *is* a one-line selection at rest, and motions manipulate it directly.

- **`OutputSelection` is a struct, not an `Inactive`/`Active` enum.** `{ anchor: usize, mode: SelectionMode, snapshot: Option<Rc<[String]>> }`, where `enum SelectionMode { Normal, Visual }`. In `Normal` the selection is the single cursor row and plain motions move it whole (anchor follows the cursor); in `Visual` (the vim visual-line sub-mode) plain motions grow the range from the fixed `anchor`. `snapshot` freezes the buffer once the selection stops following the live tail, so a streaming child can't drift a pinned range. There is always a selection — `selected_range` returns `(cursor, cursor)` in `Normal`.
- **`V` is a built-in gated to vim mode, not a rebindable action.** `OutputAction::SelectLinewise` was deleted; the action set is now `{ SelectAll, Cancel }`. `V` toggles visual mode (`toggle_visual`) only when `navigation_keys` is `ArrowsAndVim`. The editor-style gestures — Shift+Up/Down (`select_extend_up`/`down`), Ctrl+Shift+Up/Down (`select_extend_to_top`/`to_bottom`) — are always available and enter visual mode on demand. Dispatched in `input/mod.rs::dispatch_output_selection_gesture`.
- **One selection color.** The separate cursor tint (`active_focus_color`) was removed; the whole selected range — a single row or many — renders in `finder_match_bg`, forced onto every span and padded to the pane width so it covers ANSI-colored log text.
- **Vocabulary matches the keymap.** Identifiers use "visual" (`is_visual`/`enter_visual`/`toggle_visual`/`exit_visual`), matching the `V` binding and the `visual: N lines` title.

Files: `panes/pane_impls.rs` (`OutputSelection` struct + `SelectionMode` + `OutputPane` methods), `panes/output.rs` (single-color highlight, visual title), `panes/actions.rs` (`navigate_output`), `panes/pane_data/mod.rs` (`copy_payload_for_output` + ANSI strip), `keymap/actions.rs` (`OutputAction { SelectAll, Cancel }`), `integration/framework_keymap.rs` (Navigable + `CopySelection`), `input/mod.rs` (status-aware Esc + `dispatch_output_selection_gesture`), `app/mod.rs` (N-lines toast + reset on open). Tests in `app/tests/interaction.rs`.

---

## Phase 2 — mouse (2026-05-31)

Both Phase-2 mouse pieces are now shipped:

- **Wheel scroll already worked.** `input/mod.rs` routes `MouseEventKind::ScrollUp`/`ScrollDown` through `scroll_pane_at` to each pane's `Viewport`; mouse capture is already enabled (clicks focus panes and position cursors). The plan's "currently-dead wheel scroll" note (decision 2 / line 7) was stale — no work needed.
- **Drag-to-select-lines added.** Left-press already positions the output cursor at the clicked row via the existing hit-test path (`interaction.rs::handle_click` → `set_pane_pos`). The new `MouseEventKind::Drag(Left)` arm calls `handle_output_drag`, which maps the pointer to a buffer row with `Viewport::pos_to_local_row` and calls `OutputPane::select_drag_to(live, row)`: the first drag enters visual mode anchored at the press row (the cursor the press left), each subsequent drag moves the cursor end — so the range grows, flips across the anchor, and yanks through the same `selected_range`/`copy_payload` path the keyboard uses. The drag is gated on output focus, so a drag begun in another pane never starts an output selection. Off-body motion (above/below the pane) yields no row and holds the range.

Files: `panes/pane_impls.rs` (`OutputPane::select_drag_to`), `input/mod.rs` (`Drag(Left)` arm + `handle_output_drag`). Tests in `app/tests/interaction.rs` (`output_drag_selects_the_line_range_and_yanks_it`, `output_drag_ignored_when_output_not_focused`).

No Shift/Alt-drag escape hatch to native terminal selection was added — mouse capture already suppresses native selection app-wide, so drag-select takes nothing away. Deferrable if native selection is ever missed (line 44).

**Click-after-drag resets the selection.** A left-press routes through the generic hit-test (`interaction.rs::handle_click` → `set_pane_pos`), which only moves the cursor — it left visual mode and the old anchor in place, so a click after a drag extended the prior range instead of starting fresh. Now an output-body press is intercepted in `handle_mouse_click` and routed through `OutputPane::click_select_row`, which collapses to Normal mode with the anchor on the clicked line. So click-drag selects a range; release-then-click clears it and selects just the clicked line, and a following drag anchors at the new click. Test: `output_click_after_drag_clears_the_selection_to_the_clicked_line`.

Character-level (sub-line) selection — decision 3's deferred Option-D remainder — is the only output-yank item left.
