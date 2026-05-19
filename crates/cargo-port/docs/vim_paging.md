# Vim Mode and Paging Plan

## Goal

Add page-step navigation and fill out core vim-style list navigation. Wherever a
cursor/list surface supports Home / End today, add PageUp / PageDown. When vim
mode is enabled, add the list-navigation vim keys in this plan, including `g g`
for Home.

## Non-goals

- Do not change non-vim defaults beyond adding PageUp / PageDown.
- Do not add output scrollback, scroll-by-pixel, mouse-wheel page steps, or
  column-wise paging unless the Output decision below changes.
- Do not introduce a timeout for chord disambiguation. Chords use the prefix
  tree and explicit conflict rules.
- Do not add chord support beyond what the vim bindings need (`g g` is the only
  chord in this plan).
- Do not make the keymap UI capture multi-key chords in this pass unless the
  chord-editing decision below changes. Chords may be TOML-editable while still
  displayed read-only in the UI.

## User Behavior

### Always-on

- `PageUp` moves the cursor up by one page step.
- `PageDown` moves the cursor down by one page step.
- A page step is `visible_rows - 1` when the viewport has visible rows, with a
  minimum step of 1. If `visible_rows == 0`, paging is a no-op.
- Half-page steps use `visible_rows / 2` when the viewport has visible rows, with
  a minimum step of 1. If `visible_rows == 0`, half-page paging is a no-op.

Every included surface that handles Home / End also handles PageUp / PageDown:

- Project list.
- Detail panes: Package, Git, Lang, CPU, Targets.
- Lints.
- CI Runs.
- Toasts viewport.
- Finder overlay results.
- Keymap UI overlay rows.

Output is not included by default. It is currently a static, auto-tail pane
rather than a cursor/scrollback surface.

### Vim Mode (`navigation_keys = "ArrowsAndVim"`)

Additional bindings on top of the existing `h/j/k/l`:

- `g g` -> Home.
- `G` -> End.
- `ctrl-u` -> half-page up.
- `ctrl-d` -> half-page down.
- `ctrl-b` -> full-page up, same step as `pageup`.
- `ctrl-f` -> full-page down, same step as `pagedown`.

Arrow keys, `home`, `end`, `pageup`, and `pagedown` continue to work in vim
mode.

Shifted ASCII letters are canonicalized as uppercase characters internally.
Parsing `shift-g`, legacy `Shift+G`, raw `G`, and a terminal Shift+g event must
resolve to the same binding.

### Overlay and Text Input Boundaries

Finder and keymap UI already bypass normal navigation dispatch in different
ways. The default implementation rule is:

- Finder text input supports physical `PageUp` / `PageDown` for result-list
  paging.
- Keymap UI browse mode supports physical `PageUp` / `PageDown` for row paging.
- No vim chord buffering runs while typing in Finder, while the keymap UI is
  capturing a key, or while another text-input mode owns the key.
- Pending chord state is cleared on focus change, runtime-scope change, overlay
  open/close, text-input entry, config reload, and failed prefix re-evaluation.

## Remapping

Vim bindings flow through the same TOML keymap merge as every other binding.
The order becomes:

1. Defaults.
2. Vim extras.
3. TOML overlay.
4. Collision and reservation validation.

Apply this order to both app navigation extras and pane-local `vim_extras()`.
Today's order in the framework applies TOML before vim extras, which makes vim
extras hard to override from user config.

TOML arrays continue to mean alternate bindings. Chord steps are represented by
a single string with space-separated steps, such as `g g`.

## Keybinding Terminology (Zed-aligned)

Match Zed's keymap.json conventions:

- Modifier separator: hyphen, lowercase. Examples: `ctrl-k`, `shift-tab`,
  `alt-enter`.
- Modifier order in display: `ctrl-alt-shift-<key>`.
- Key names match Zed word for word: `escape`, `enter`, `tab`, `backspace`,
  `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`,
  `right`, `space`, `f1`..`f12`.
- Chord steps: space-separated. Examples: `g g`, `ctrl-k ctrl-s`.

Parser compatibility is broad: accept Zed style (`ctrl-k`), existing legacy
style (`Ctrl+K`), and shifted-letter spellings (`shift-g`, `Shift+G`) where
they describe the same physical key. Display emits the new style in TOML-facing
surfaces, subject to the status-bar display decision below.

This repo currently has two keybinding stacks:

- Framework keymap: `tui_pane::KeyBind`.
- Legacy app keymap: `src/tui/keymap::KeyBind`.

The syntax migration must update both parsers/renderers, generated default TOML,
saved keymap output, keymap UI reload, and tests/assets before any UI emits only
Zed-style strings. Otherwise the keymap UI can write a string that the legacy
reload path cannot parse.

## Implementation Phases

### Phase 1 - Key sequences and chord state

Goal: extend the framework keymap so a binding can be a sequence of `KeyBind`s,
not just one key.

- Introduce `KeySequence(Vec<KeyBind>)`.
- Add `KeySequence::parse` and display. `KeyBind::parse` remains the single-key
  parser.
- Single-key bindings stay ergonomic via `From<KeyBind>` / `From<KeyCode>` /
  `From<char>`.
- Update `Bindings`, `ScopeMap`, TOML overlay parsing, reverse lookup, collision
  checks, `RuntimeScope`, bar slots, keymap UI rows, and saved keymap output to
  carry `KeySequence`.
- Keep TOML arrays as alternate sequences:
  - `home = "g g"` means one two-step sequence.
  - `home = ["home", "g g"]` means two alternate sequences.
- Store pending chord state outside the immutable resolved `ScopeMap`. The state
  owner should be the framework/input-session layer, keyed by runtime scope.
- Dispatcher step for a key:
  1. Append the incoming `KeyBind` to that scope's pending buffer.
  2. If the buffer matches a complete binding and is not an unresolved longer
     prefix, dispatch and clear.
  3. Else if the buffer is a prefix of a binding, keep it and consume the key.
  4. Else clear the buffer and re-evaluate the current key as a fresh sequence
     of length 1.
- Same-scope prefix conflicts, cross-scope priority, and reserved vim keys are
  pending product decisions below.

### Phase 2 - Page-step vocabulary

- `Viewport::page_up()` / `page_down()`: move by `page_step()`.
- `Viewport::half_page_up()` / `half_page_down()`: move by `half_page_step()`.
- `page_step()` returns `None` when `visible_rows == 0`; otherwise
  `Some(visible_rows.saturating_sub(1).max(1))`.
- `half_page_step()` returns `None` when `visible_rows == 0`; otherwise
  `Some((visible_rows / 2).max(1))`.
- `ListNavigation::PageUp` / `PageDown` / `HalfPageUp` / `HalfPageDown`
  variants.
- `Navigation` trait: add `PAGE_UP`, `PAGE_DOWN`, `HALF_PAGE_UP`,
  `HALF_PAGE_DOWN` consts; extend the default `list_navigation()` mapper.
- `NavigationAction`: add `PageUp`, `PageDown`, `HalfPageUp`, `HalfPageDown`
  variants. Default bindings:
  - `KeyCode::PageUp` -> `PageUp`.
  - `KeyCode::PageDown` -> `PageDown`.
  - Half-page actions: no default key outside vim mode.

### Phase 3 - Dispatch wiring

- `src/tui/panes/actions.rs`: extend ProjectList, detail panes, Lints, CI Runs,
  and Toasts dispatch to route the four new actions to page / half-page methods.
- `src/tui/finder/dispatch.rs`: handle physical PageUp / PageDown in the Finder
  result list.
- `src/tui/keymap_ui/mod.rs`: handle physical PageUp / PageDown in browse mode.
- Toasts navigation: extend `ListNavigation` matching.
- Add key-capture reservation for physical PageUp / PageDown if they remain
  reserved navigation keys.

### Phase 4 - Vim extras and remap order

In `apply_vim_navigation_extras`, when `VimMode::Enabled`, append:

- `KeySequence([g, g])` -> Home.
- `G` -> End.
- `ctrl-u` -> HalfPageUp.
- `ctrl-d` -> HalfPageDown.
- `ctrl-b` -> PageUp.
- `ctrl-f` -> PageDown.

Existing `h/j/k/l` stay in place.

Flip both registration paths so the TOML overlay applies after vim extras:

- `register_navigation`: defaults -> vim extras -> TOML overlay.
- `build_pane_bindings`: defaults -> pane `vim_extras()` -> TOML overlay.

Update loader, keymap UI, and style docs for the expanded vim reservation set
after the reservation decision is recorded.

### Phase 5 - Zed-style key syntax

- Replace `with_modifier_prefix` / `with_short_modifier_prefix` with renderers
  that support the selected display policy.
- Replace key-name mappings with Zed's spellings (`escape`, `pageup`,
  `pagedown`, `backspace`, etc.).
- Extend both framework and legacy parsers to accept Zed style and legacy style.
- Normalize shifted-letter spellings so `shift-g`, `Shift+G`, raw `G`, and
  terminal Shift+g all resolve to the same `KeySequence`.
- Update generated keymap comments/defaults, keymap UI save/reload, status/keymap
  display, `tests/assets/default-keymap.toml`, and tests that assert key-string
  output.

### Phase 6 - Keymap UI

- Surface the new page actions in the keymap help overlay.
- Display chord sequences correctly.
- Keep chord editing TOML-only unless the chord-editing decision approves
  multi-step capture.
- If inactive vim rows should be visible when vim mode is off, add an explicit
  inactive/default vim-binding source. The current active keymap cannot provide
  rows for bindings that were never registered.

## Tests

### Framework (`tui_pane`)

- `KeySequence::parse` / display round-trip for single keys and chords.
- TOML string `g g` parses as one sequence; TOML arrays parse as alternate
  sequences.
- Dispatcher: single-key binding fires on the first press.
- Dispatcher: chord `g g` does not fire on the first `g`; fires on the second.
- Dispatcher: `g` followed by an unrelated key clears the buffer and dispatches
  the unrelated key as fresh when applicable.
- Dispatcher: `g` followed by a second key that has no binding leaves no
  residual state.
- Pending state clears on focus change, scope change, overlay open/close,
  text-input entry, and config reload.
- Buffer is per-scope: a pending `g` in the nav scope does not affect globals.
- `Viewport::page_up` / `page_down` step by `visible_rows - 1`, clamp at bounds,
  no-op when the list is empty, and no-op when `visible_rows == 0`.
- `Viewport::half_page_up` / `half_page_down` step by `visible_rows / 2`, clamp
  at bounds, no-op when `visible_rows == 0`, and step at least 1 when rows are
  visible.
- `ListNavigation::PageUp` etc. route through `Toasts::on_navigation` and the
  viewport changes by the expected delta.

### Binary

- Default keymap includes `KeyCode::PageUp` / `PageDown` for `PageUp` /
  `PageDown` actions.
- With `navigation_keys = "Arrows"`, `g` and `G` are unbound.
- With `navigation_keys = "ArrowsAndVim"`, `g g` dispatches Home, `G` dispatches
  End, `ctrl-u` / `ctrl-d` dispatch half-page actions, and `ctrl-b` / `ctrl-f`
  dispatch full-page actions.
- Shifted-letter normalization covers raw `G`, terminal Shift+g, TOML `shift-g`,
  and legacy `Shift+G`.
- Explicit PageUp / PageDown matrix:
  - ProjectList.
  - Package.
  - Git.
  - Lang.
  - CPU.
  - Targets.
  - Lints.
  - CI Runs.
  - Toasts.
  - Finder.
  - Keymap UI.
- Each matrix entry asserts deltas, clamping, empty-list behavior, and small-list
  behavior.
- TOML overlay overrides a vim extra after the registration-order change, subject
  to cross-scope priority rules.
- Keymap capture rejects PageUp / PageDown as reserved navigation keys if that
  reservation policy is kept.

### Keybinding Syntax

- `KeyBind::parse("ctrl-k")` parses identically to `KeyBind::parse("Ctrl+K")`
  in both keymap stacks.
- `KeySequence::parse("g g")` parses as two `g` steps.
- `KeySequence::display()` emits the selected Zed-style form.
- Round-trip: display output parses back to the same `KeySequence`.
- Every Zed key name (`escape`, `pageup`, `backspace`, `f7`, etc.) parses to the
  expected `KeyCode`.
- Generated default keymap TOML and keymap UI save output reload through both the
  framework and legacy parsers.

## Implementation Defaults

These came out of review, but they are not worth stopping for product review
unless implementation uncovers a concrete conflict.

- Output stays out of this plan. It is static/auto-tail today; real output
  scrollback should get its own plan.
- No chord buffering runs in Finder text input, key capture, or other text-input
  modes. Physical PageUp / PageDown remain overlay-local there.
- When vim mode is off, the keymap UI shows active bindings only. It should not
  invent disabled vim rows without a real inactive-binding source.
- Chords display in the keymap UI, but editing them is TOML-only in this pass.
  Multi-step capture can be a later feature.
- Existing compact status-bar glyphs/labels stay where they already exist.
  Zed-style strings are for TOML, generated config, and the keymap UI.
- The doc uses "core vim-style list navigation" rather than "complete vim mode"
  or "full vim support".

## Remaining Product Decisions

These are the user-facing decisions worth confirming.

1. Chord and vim-key priority:
   - Recommendation: reject same-scope prefix conflicts, reserve default vim
     navigation keys in vim mode, and document that higher-priority cross-scope
     bindings must be moved before a navigation remap can use their keys.
   - Decision: same-scope override is allowed, but cross-scope conflicts are
     reported clearly instead of silently shadowing vim chords or reserved vim
     navigation keys.
