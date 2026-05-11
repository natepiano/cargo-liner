# Phase 16 — significant findings (deferred for review)

Source: Phase 16 `/phase-review` of `docs/tui-pane-lib.md` on 2026-05-10. Three findings the architect subagent flagged as `significant` (changing scope, ordering, or architectural intent — needs user approval before applying to the plan). The eight `minor` findings from the same review have already been folded directly into `docs/tui-pane-lib.md`.

Each finding has the four required sub-sections: **What the plan currently says** / **What just shipped** / **Why it matters** / **The proposed plan change**.

---

## Finding 2 — Phase 18 "Wire dispatchers" inventory does not name the `KeyBind` type-consolidation work

**What the plan currently says.** Phase 18's "Wire dispatchers" sub-section (lines ~3207-3215) and `Delete:` block (lines ~3221-3246) name many specific types and call sites but do not name the `KeyBind` consolidation. The Phase 16 retrospective at line ~3013 records the divergence but does not commit Phase 18 to any specific consolidation choice.

**What just shipped.** `crate::keymap::KeyBind` and `tui_pane::KeyBind` are distinct types: the former normalises uppercase Char + SHIFT → uppercase, the latter is plain. Phase 16 introduced four `framework_bind` shims at `src/tui/input.rs:651-654`, `src/tui/panes/actions.rs:109-112, 228-231, 325-328` to materialise a `tui_pane::KeyBind` from a `crate::keymap::KeyBind` per call. These shims exist purely to bridge the legacy and framework paths through the same `handle_*_key` body.

**Why it matters.** When Phase 18 retires the legacy dispatch reads, every `crate::keymap::KeyBind` site either (a) deletes (good — the shim becomes redundant), (b) flips to `tui_pane::KeyBind` (rename whose correctness depends on normalisation parity — every Shift+letter rebind round-trip relies on `KeyBind::new`'s uppercase-strip behavior, and the framework type does not perform that normalisation today), or (c) survives in `keymap_ui.rs` / `settings.rs` for non-keymap-builder reasons. The plan currently does not name which sites end up in (a) vs (b) vs (c), so the implementer can't tell whether the four shims should be deleted, flipped, or absorbed into a larger Phase 18 consolidation. Without naming this, the cutover changeset is one TODO away from a Shift+letter rebind regression.

**The proposed plan change.** Add a bullet to Phase 18's "Wire dispatchers" sub-section (right before "Atomic cutover constraint" at line ~3217):

> **`KeyBind` type consolidation at the dispatch boundary.** Phase 18 picks one `KeyBind` representation for the post-cutover binary. **Decision: `tui_pane::KeyBind`** — the framework type is the dispatch-side representation, and the binary's `crate::keymap::KeyBind` only exists today because `ResolvedKeymap` consumes it. Concrete: every `crate::keymap::KeyBind::*` call site that survives Phase 18 (the audit list overlaps with the `crate::keymap::GlobalAction` audit one bullet up) flips to `tui_pane::KeyBind`. The four `framework_bind = tui_pane::KeyBind { code: bind.code, mods: bind.modifiers }` shims at `src/tui/input.rs:651-654` and `src/tui/panes/actions.rs:109-112, 228-231, 325-328` delete (the local `bind` is already `tui_pane::KeyBind`). **Normalisation parity verification:** before deletion, audit `crate::keymap::KeyBind::new`'s normalisation (uppercase Char + SHIFT → uppercase, BackTab → Tab + SHIFT). If `tui_pane::KeyBind` does not perform the same normalisation, port it into `tui_pane::KeyBind`'s constructors as part of this Phase 18 work — it is a user-visible behavior parity gap.

---

## Finding 3 — Phase 19 funnel test prerequisites need the residual `app.focus.set(...)` call sites enumerated

**What the plan currently says.** Phase 18's `Delete:` block at line ~3245 deletes "the `self.focus.set(id.to_legacy())` mirror line inside `App::set_focus`". Phase 19's funnel-test acceptance bullet at line ~3289 says "A test impl that overrides `set_focus` to count calls observes every framework focus change (NextPane/PrevPane, OpenKeymap, OpenSettings, return-from-overlay)". Neither phase enumerates the residual `app.focus.set(...)` call sites that need to migrate to `app.set_focus(FocusedPane::App(...))` for the funnel-test assertion to hold.

**What just shipped.** A `git grep 'app\.focus\.set(' src/` finds 15 non-test call sites outside `framework_keymap.rs::set_focus` itself: `src/tui/finder.rs:653`, `src/tui/interaction.rs:29, 45`, `src/tui/input.rs:118, 353, 359`, `src/tui/app/mod.rs:333, 653, 835, 840, 855, 860, 908`, `src/tui/panes/actions.rs:386`, `src/tui/app/async_tasks/tree.rs:36, 66`. The most user-visible: mouse-click handlers (`input.rs:353, 359` — pane-region click sets `app.focus`), the `focus_next_pane` / `focus_previous_pane` cycle (`app/mod.rs:840, 860`), and the structural Esc-on-output return-to-Targets (`input.rs:118`).

**Why it matters.** The funnel-test assertion ("set_focus is the single funnel for framework focus changes") fires only if every other writer has been re-routed. If even one of those 15 sites still writes `app.focus.set(PaneId::X)` directly, the test passes vacuously (the override counter is incremented for the framework-routed paths, but the legacy field also moves through the unmigrated site and the funnel claim is false). Phase 19 cannot "begin" without this enumeration; today it would silently ship a false-positive test. Phase 18's `Delete:` block also doesn't tell the implementer which of these sites delete with the legacy `app.focus` field versus which migrate to `app.set_focus`.

**The proposed plan change.** Add a new bullet to Phase 18's `Delete:` block (right after the existing `app.focus` mirror-line bullet at line ~3245), and a parallel acceptance prerequisite bullet at the head of Phase 19's funnel-test section:

> **(Phase 18 `Delete:` block — new bullet)** **Residual `app.focus.set(...)` call site migration.** With the legacy `app.focus: Focus` field deleted, the 15 non-test call sites that write `app.focus.set(PaneId::X)` must each migrate or delete. Specifically: `finder.rs:653`, `interaction.rs:29, 45`, `input.rs:118, 353, 359`, `app/mod.rs:333, 653, 835, 840, 855, 860, 908`, `panes/actions.rs:386`, `app/async_tasks/tree.rs:36, 66`. Each site flips to one of (a) `app.set_focus(FocusedPane::App(AppPaneId::X))` for genuine focus changes through the framework funnel; (b) deletion if the site is dead after the cutover; (c) `app.framework.overlay_layer_*` calls if the site is overlay open/close (those flow through `Framework::open_overlay` / `close_overlay`, not focus). The mouse-click handlers (`input.rs:353, 359`) and `focus_next_pane` / `focus_previous_pane` (`app/mod.rs:840, 860`) are the highest-risk migrations; the structural Esc-on-output return-to-Targets at `input.rs:118` deletes alongside the Phase 17 structural-Esc preflight rewrite.
>
> **(Phase 19 — new prerequisite at head of funnel-test section, before the existing acceptance bullet at line ~3289)** **Acceptance prerequisite.** Phase 18's `Residual app.focus.set(...) call site migration` bullet must already have landed. Without it, the funnel-test assertion passes vacuously — the override counter increments for framework-routed paths but residual sites still write `app.focus` directly, and the "single funnel" claim is false. Phase 19 implementer verifies before authoring the test: `git grep 'app\.focus\.set(' src/` (excluding `framework_keymap.rs::set_focus`) returns zero hits.

---

## Finding 5 — Phase 18 atomic-cutover changeset is now larger than the plan acknowledges; needs a single canonical entry-flow inventory

**What the plan currently says.** Phase 18's "Wire dispatchers" sub-section (lines ~3207-3215) lists the framework-side dispatcher bodies. The "Atomic cutover constraint" note at line ~3217 says "The dispatcher bodies and the deletion of the legacy dispatch reads must land together in the same Phase 18 cutover changeset." The `Delete:` block (lines ~3221-3246) lists deletions in roughly file order. No single block enumerates the **entry-point swap** in `src/tui/input.rs::handle_key_event` — the eight legacy entry points at lines 122-152 that all need to flip atomically.

**What just shipped.** `src/tui/input.rs::handle_key_event` (lines 90-155) currently dispatches in this order: structural Esc preflight (lines 95-121, two arms), `handle_confirm_key` (line 122), `handle_overlay_editor_key` (line 125), three `app.overlays.is_*_open()` short-circuits (lines 128, 132, 136), `handle_global_key` (line 140), then the `match panes::behavior(...)` arms calling `handle_detail_key` / `handle_lints_key` / `handle_ci_runs_key` / `handle_normal_key` / `handle_toast_key` (lines 144-153). After Phase 18, this entry-point flow becomes: structural Esc preflight (Phase 17 form), `handle_confirm_key` (untouched — modal confirmation, not framework-routed), framework dispatch chain (overlays → globals → focused-pane → navigation → app-pane scope → `dismiss_chain`).

**Why it matters.** The Phase 18 implementer reading the plan today has to reconstruct the post-cutover entry-flow by cross-referencing the "Wire dispatchers" inventory, the `Delete:` block, the Phase-15-deferred (a)/(b)/(c) bullets, the Phase 17 structural-Esc preflight rewrite, and the Phase 19 funnel prerequisites. **`handle_confirm_key` is not addressed at all** — the modal confirmation system is orthogonal to framework keymap dispatch but still lives in the same `handle_key_event` entry, and the plan neither preserves it nor migrates it. The atomic-cutover constraint requires a one-PR swap; without a canonical pre-cutover-vs-post-cutover entry-flow diagram, the implementer either ships a partial swap (failure mode: double-dispatch or dead key) or burns time reconstructing the entry-flow at PR-draft time.

**The proposed plan change.** Add a new sub-section "Phase 18 entry-flow inventory" between the existing "Atomic cutover constraint" note and the `Delete:` block. The sub-section lays out the pre-cutover and post-cutover entry-flow side by side:

> **Phase 18 entry-flow inventory.** Pre-cutover `src/tui/input.rs::handle_key_event` dispatches in this order (lines 90-155):
>
> 1. Structural Esc preflight: kill running example (line 95-111), then clear `example_output` + return-to-Targets (line 112-119).
> 2. `handle_confirm_key(app, code)` (line 122) — modal confirmation gate (`y`/`n` for Clean confirms).
> 3. `handle_overlay_editor_key(app, &normalized)` (line 125) — `OpenEditor` global from inside Settings/Keymap overlays.
> 4. Three overlay short-circuits: `keymap_ui::handle_keymap_key` (line 128), `finder::handle_finder_key` (line 132), `settings::handle_settings_key` (line 136).
> 5. `handle_global_key(app, &normalized)` (line 140) — 11-arm match on `crate::keymap::GlobalAction::*`.
> 6. Per-pane `match panes::behavior(...)` arms: `handle_detail_key`, `handle_lints_key`, `handle_ci_runs_key`, `handle_toast_key`, `handle_normal_key`.
>
> Post-cutover `handle_key_event` dispatches in this order:
>
> 1. Structural Esc preflight (Phase 17 form, reverse-lookup against `OutputAction::Cancel`).
> 2. `handle_confirm_key(app, code)` — **survives untouched**. Modal confirmation is binary-specific behavior, not framework keymap dispatch; it stays at this position.
> 3. Framework dispatch chain: `Framework::overlay()` → if `Some`, route through `framework.settings_pane.handle_key` / `framework.keymap_pane.handle_key` / `Mode::TextInput(finder_keys)` from `FinderPane::mode()` (replaces step 4 above).
> 4. Framework globals chain: `app.framework_keymap.framework_globals().action_for(&bind)` → dispatch via `tui_pane::GlobalAction::dispatcher()`. Then `app_globals` scope → `AppGlobalAction::dispatcher()`. (Replaces step 5 above; `handle_overlay_editor_key`'s body absorbs into `AppGlobalAction::dispatcher()` when the overlay context resolves to Settings/Keymap.)
> 5. Per-pane dispatch: `app.framework_keymap.dispatch_app_pane(focused_app_pane_id, &bind, app)` — fires the registered pane's `Shortcuts::dispatcher`. (Replaces the `match panes::behavior(...)` arms calling per-pane wholesalers.)
> 6. Navigation scope fallback: `app.framework_keymap.navigation::<AppNavigation>().action_for(&bind)` → dispatch via `AppNavigation::dispatcher()`. (Phase 16 routed Up/Down/Home/End through navigation lookup *after* pane scope; Phase 18 lifts that out of the per-pane wholesalers and into the entry-flow.)
> 7. Toasts focus gate (Phase-15-deferred (b)): when `Framework::focused() == FocusedPane::Framework(FrameworkFocusId::Toasts)`, the six-step chain from Phase 15 fires. Replaces the `PaneBehavior::Toasts → handle_toast_key` arm.
>
> The deletion list in the `Delete:` block enumerates which legacy bodies / call sites disappear; this inventory tells the implementer what the resulting `handle_key_event` looks like after they're applied. **`handle_confirm_key` is intentionally preserved at step 2** — modal confirmation is not framework keymap dispatch, and Phase 18 does not migrate it.
