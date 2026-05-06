# Shortcut bar — collapse into a keymap renderer

## The problem

`src/tui/shortcuts.rs` duplicates information that already lives in `src/keymap.rs`, hardcodes `"Enter"` (defeating user rebindings), and in one place advertises an action the dispatcher doesn't perform.

Four symptoms:

1. **`App::enter_action`** at `src/tui/app/query/post_selection.rs:59` answers "what does Enter do here?" by branching on `InputContext` and reading state across Focus, Panes, ProjectList, and Ci. Sole consumer: the status-bar renderer.

2. **`shortcuts::enter()`** at `src/tui/shortcuts.rs:107` hardcodes the literal string `"Enter"`. Every scope already has an `Activate` action with a configurable binding (`PackageAction::Activate`, `CiRunsAction::Activate`, etc.).

3. **`shortcuts::for_status_bar`** at `src/tui/shortcuts.rs:120` takes 7 app-derived parameters and threads them through 4 per-context helpers. Each helper re-implements "what shortcuts apply in this context" — already enumerated by the per-scope `*Action` enums.

4. **CiRuns "fetch" label is a lie.** `enter_action` returns `Some("fetch")` when the cursor is past the last run. The dispatcher (`src/tui/panes/actions.rs:194,217`) only fires `handle_ci_enter` when `visible_runs.get(cursor_pos)` is `Some`. At EOL, Enter does nothing. The bar advertises a no-op.

## The duplication, named

| Where the structural truth lives | Where the renderer re-encodes it |
|---|---|
| `<Scope>Action::ALL` (`PackageAction`, `GitAction`, `CiRunsAction`, `LintsAction`, `ProjectListAction`, `TargetsAction`, `GlobalAction`) — the action set per scope | The per-context helper fns, each pushing hardcoded `Shortcut`s |
| `ScopeMap::display_key_for(action) -> String` (`keymap.rs:343`) — bound key | `Shortcut::fixed("Enter", _)` and other hardcoded key names |
| `<Scope>Action::Activate` — the "primary action" in each scope | `App::enter_action` |

Two `InputContext` matches run on the same value: one inside `App::enter_action`, one inside `for_status_bar`.

Stale parameter naming: `for_status_bar`'s `is_rust: bool` parameter is set at `render.rs:543` to `app.project_list.clean_selection().is_some()` — i.e. **clean availability, not Rust-ness.** The new architecture surfaces this as `ProjectList::clean_available()`, called from each per-pane impl.

Bogus `const fn`: `Shortcut::from_keymap` and `Shortcut::disabled_from_keymap` (`shortcuts.rs:80, 88`) are declared `const fn` but take `String` — `const fn` cannot construct `String`. Pre-existing defect, drop the `const` in cleanup.

## Macro-exposed API confirmed

The `action_enum!` macro at `src/keymap.rs:215-250` exposes for every action enum:
- `pub const ALL: &[Self]` — the full variant list, in **enum-declaration order**. Suitable for iteration only when render order matches declaration order; otherwise impls must specify their own ordered slice.
- `pub const fn description(self) -> &'static str` — the long description (`"Open URL or Cargo.toml"`, `"Toggle branch/all filter"`).
- `pub const fn toml_key(self) -> &'static str`.
- `from_toml_key(&str)`.

`description()` is the **long** form. Today's bar uses **short** labels (`"open"`, `"branch/all"`, `"fetch more"`). The new architecture must NOT fall back to `description()` for bar labels — the long form would overflow the bar's available width. Every per-pane impl returns its own short label or `None`.

## Target architecture

The shortcut bar is a thin renderer over the keymap. It contributes nothing structural — every action it shows is named in `keymap.rs`; every key it shows comes from `display_key_for(action)`.

### Five rules

1. **Dispatch by `PaneId`, not `InputContext`, for action-bound rows.** `InputContext::DetailFields` covers two scopes (`PackageAction` when Package is focused, `GitAction` when Git is focused). The unambiguous mapping is `app.focus.base() -> PaneId -> bar_renderer()`. `InputContext` still gates: (a) overlay vs. base contexts, (b) whether globals are shown at all, (c) the static-only arms (`Finder`, `Settings`, `Keymap*`, `Toasts`, `Output`).

2. **Every action displayed in the bar is produced by a `BarRenderer::render_into` call.** No special-case free functions. Per-pane content goes through the focused pane's renderer; globals go through `GlobalActions`'s renderer. One contract.

3. **For each action, ask `ResolvedKeymap` for the bound key.** Every key string in action-bound rows comes from `km.<scope>.display_key_for(action)`. No `Shortcut::fixed("Enter", _)` for action rows.

4. **Render order is per-impl.** `<Action>::ALL` is enum-declaration order, which won't always match the bar's left-to-right order (today's globals strip uses Find/editor/terminal/settings/keymap/rescan/quit/restart, but `GlobalAction::ALL` is Quit/Restart/Find/…). Each impl that needs a non-declaration order defines its own ordered slice and iterates it.

5. **Shared predicates live on the data type, not as free functions.** `*Action::Clean`'s gate (`clean_selection().is_some()`) is shared across multiple panes. Add a named accessor `ProjectList::clean_available()` and have every per-pane impl call it. Same rule applies to any future shared predicate.

### Trait surface

The trait, the dispatch accessor, and the `GlobalActions` impl all live in a new module **`src/tui/app/bar_renderer.rs`**. That keeps the bar-rendering machinery in one file, off `app/mod.rs`, and easy to find. Module is declared in `src/tui/app/mod.rs` as `mod bar_renderer;` (private to `app/`); the trait itself is `pub(crate)` so pane host files in sibling `tui::*` modules can write `impl BarRenderer for …`. `Shortcut` stays `pub(super)` — still reachable from pane modules under `crate::tui::*`.

```rust
pub(crate) trait BarRenderer {
    /// Push this renderer's contribution into the supplied vecs.
    /// `nav` carries left-side navigation hints; `actions` carries the
    /// center action strip; `globals` carries the right-side global strip.
    /// Each impl pushes only into the vecs it owns content for.
    fn render_into(
        &self,
        app: &App,
        km: &ResolvedKeymap,
        nav: &mut Vec<Shortcut>,
        actions: &mut Vec<Shortcut>,
        globals: &mut Vec<Shortcut>,
    );
}
```

Single method. Object-safe out of the box (no associated types, no generics). `&dyn BarRenderer` works directly; no wrapper trait needed.

Each impl decides:

- which vecs to push to,
- in what order,
- with which keys (from `km`) and labels (constant or state-conditional),
- with which `ShortcutState` (Enabled or Disabled).

### Recommended impl convention

Each `BarRenderer` impl follows the same internal pattern, even though the trait doesn't enforce it (the trait is one method; the convention is what makes them uniform):

```rust
impl BarRenderer for PackagePane {
    fn render_into(&self, app: &App, km: &ResolvedKeymap, nav, actions, _globals) {
        nav.push(NAV);
        nav.push(TAB_PANE);
        for &action in PackageAction::ALL {
            let Some(label) = self.bar_label(action, app) else { continue };
            let key = km.package.display_key_for(action);
            let state = if self.enabled(action, app) { Enabled } else { Disabled };
            actions.push(Shortcut { key: Cow::Owned(key), description: label, state });
        }
    }
}

impl PackagePane {
    fn bar_label(&self, action: PackageAction, app: &App) -> Option<&'static str> {
        match action {
            PackageAction::Activate => self.activate_label(app),
            PackageAction::Clean    => Some("clean"),
        }
    }

    fn enabled(&self, action: PackageAction, app: &App) -> bool {
        match action {
            PackageAction::Clean => app.project_list.clean_available(),
            _                     => true,
        }
    }

    fn activate_label(&self, app: &App) -> Option<&'static str> {
        // logic moved from App::enter_action's DetailFields/Package branch
    }
}
```

`bar_label` returns `None` to skip a row, `Some("…")` to render with that label. The match-on-action body keeps the per-action logic together; `enabled` and `activate_label` stay private to the impl. Every per-pane impl follows the same internal layout.

### Globals as a `BarRenderer` (in `app/bar_renderer.rs`)

Globals get the same trait, not a special-case path. Define a unit struct that implements `BarRenderer`, in the same `app/bar_renderer.rs` module:

```rust
pub struct GlobalActions;

impl GlobalActions {
    /// Render order for the right-hand globals strip. Hand-ordered because
    /// `GlobalAction::ALL` is enum-declaration order, which doesn't match.
    const RENDER_ORDER: &[GlobalAction] = &[
        GlobalAction::Find,
        GlobalAction::OpenEditor,
        GlobalAction::OpenTerminal,
        GlobalAction::Settings,
        GlobalAction::OpenKeymap,
        GlobalAction::Rescan,
        GlobalAction::Quit,
        GlobalAction::Restart,
    ];

    fn bar_label(&self, action: GlobalAction) -> &'static str {
        match action {
            GlobalAction::Find         => "find",
            GlobalAction::OpenEditor   => "editor",
            GlobalAction::OpenTerminal => "terminal",
            GlobalAction::Settings     => "settings",
            GlobalAction::OpenKeymap   => "keymap",
            GlobalAction::Rescan       => "rescan",
            GlobalAction::Quit         => "quit",
            GlobalAction::Restart      => "restart",
            // not in RENDER_ORDER, never reached:
            GlobalAction::Dismiss
            | GlobalAction::NextPane
            | GlobalAction::PrevPane => unreachable!(),
        }
    }

    fn enabled(&self, action: GlobalAction, app: &App) -> bool {
        match action {
            GlobalAction::OpenTerminal =>
                app.config.terminal_command_configured()
                && !app.project_list.selected_project_is_deleted(),
            GlobalAction::OpenEditor =>
                !app.project_list.selected_project_is_deleted(),
            _ => true,
        }
    }
}

impl BarRenderer for GlobalActions {
    fn render_into(&self, app, km, _nav, _actions, globals) {
        for &action in Self::RENDER_ORDER {
            let key = km.global.display_key_for(action);
            let state = if self.enabled(action, app) { Enabled } else { Disabled };
            globals.push(Shortcut { key: Cow::Owned(key), description: self.bar_label(action), state });
        }
    }
}
```

The bar consults `GlobalActions` the same way it consults the focused pane's renderer — `render_into(app, km, …)`, push to vecs. One trait, one signature. The `RENDER_ORDER` slice gives explicit ordering distinct from `GlobalAction::ALL`.

`Dismiss`, `NextPane`, `PrevPane` are not in the right-hand strip today — they're either keymap-only bindings or surfaced in static-only arms (Toasts uses `Dismiss`). They're omitted from `RENDER_ORDER` and `unreachable!()` in the label match.

### Context-gating for globals

Globals are not shown in every context. Today's gate at `shortcuts.rs:166-167`:

```rust
let global = if context.is_overlay() || context.is_text_input() {
    vec![]
} else {
    // … emit globals
};
```

Keep the gate **external to `GlobalActions::render_into`**: the bar only calls the global renderer when `!context.is_overlay() && !context.is_text_input()`. Don't push the predicate into `GlobalActions::enabled` — that conflates "row hidden by context" with "action disabled by app state."

```rust
let mut globals = Vec::new();
if !context.is_overlay() && !context.is_text_input() {
    GlobalActions.render_into(app, km, &mut nav, &mut actions, &mut globals);
}
```

### Dispatch — `App::bar_renderer_for_focus()` (in `app/bar_renderer.rs`)

The codebase does **not** today have a `&dyn Pane` accessor on `App`. Pane structs are scattered across host fields: `app.panes.package`, `app.panes.git`, `app.panes.targets`, `app.ci` (`Ci` struct in `ci_state.rs:272`, focused as `PaneId::CiRuns`), `app.lint` (`Lint` struct in `lint_state.rs:214`, focused as `PaneId::Lints`), `app.project_list`. Pane render is dispatched today via per-`PaneId` match arms in `render.rs:405-450` calling typed functions; there is no `match PaneId -> &dyn Pane` accessor to reuse.

Rather than extend the `Pane` trait at `pane/dispatch.rs:38` (which would force adding `bar_renderer` to every pane impl plus inventing `App::focused_pane()` to traverse the scattered hosts), add a dedicated single-purpose accessor on `App`:

```rust
impl App {
    pub(super) fn bar_renderer_for_focus(&self) -> Option<&dyn BarRenderer> {
        match self.focus.base() {
            PaneId::Package     => Some(&self.panes.package),
            PaneId::Git         => Some(&self.panes.git),
            PaneId::Targets     => Some(&self.panes.targets),
            PaneId::CiRuns      => Some(&self.ci),
            PaneId::Lints       => Some(&self.lint),
            PaneId::ProjectList => Some(&self.project_list),
            _ => None,
        }
    }
}
```

The match owns the only `PaneId -> pane host` lookup the bar needs. It's one place, named for its purpose, and doesn't bloat the existing `Pane` trait. Each of the six host fields implements `BarRenderer` directly (alongside their existing `Pane` impl).

Panes without a scope enum (`Lang`, `Cpu`, `Output`, plus all overlay/static panes) fall through to `None`; the bar emits an empty action group for them or routes to a static-only arm.

**Borrow story.** `bar_renderer_for_focus` returns `&dyn BarRenderer` borrowed from `&self`. The bar then calls `renderer.render_into(app, km, …)` with another `&App` reference. Both are shared borrows of `App`, so the double-borrow type-checks. No `&mut` paths involved.

### Pane coverage table

The "host struct" column is the actual struct that implements `BarRenderer`. Some panes' host structs are not `*Pane`-named (`Ci`/`Lint` are subsystem structs that play the pane role for `PaneId::CiRuns`/`Lints`).

| `PaneId` | Has scope enum? | Host struct | impl location |
|---|---|---|---|
| `ProjectList` | `ProjectListAction` | `ProjectList` | `src/tui/project_list.rs` |
| `Package` | `PackageAction` | `panes::PackagePane` (or wherever today's `impl Pane for X` for Package lives) | alongside the existing `Pane` impl |
| `Git` | `GitAction` | the Git-pane struct (same — sibling of the existing `Pane` impl) | sibling file |
| `Targets` | `TargetsAction` | the Targets-pane struct | sibling file |
| `CiRuns` | `CiRunsAction` | `Ci` (`src/tui/ci_state.rs:272`) | `ci_state.rs` |
| `Lints` | `LintsAction` | `Lint` (`src/tui/lint_state.rs:214`) | `lint_state.rs` |
| `Lang` | none | n/a | `bar_renderer_for_focus` returns `None` |
| `Cpu` | none | n/a | `bar_renderer_for_focus` returns `None` |
| `Output` | none | n/a | `bar_renderer_for_focus` returns `None` |
| `Toasts` | none (uses `GlobalAction::Dismiss`) | n/a | static-only arm in `for_status_bar` |
| `Finder` / `Settings` / `Keymap` | none | n/a | static-only arms |

### Static-only arms

Not every focused state has a scope action enum. These render fixed action lists today and have no per-context enum:

- `InputContext::Finder`, `Settings`, `SettingsEditing`, `Keymap`, `KeymapAwaiting`, `KeymapConflict` — overlay contexts.
- `InputContext::Toasts`, `Output` — base contexts without action enums.

These stay as inline arms in `for_status_bar` that emit hardcoded `Shortcut::fixed(...)` rows. They are not action-bound and don't participate in the keymap-renderer rule.

`InputContext::Keymap` is special: the focused pane is `PaneId::Keymap` but `is_overlay()` excludes Keymap. The bar dispatches via `InputContext` for the static-only arms, sidestepping the asymmetry. Document the asymmetry where it lives (`is_overlay()` vs `InputContext::is_overlay()`); don't try to reconcile it in this refactor.

### Shared predicates on the data type

`*Action::Clean`'s gate is shared across multiple panes. Add the named accessor:

```rust
// in src/tui/project_list.rs
impl ProjectList {
    pub(super) fn clean_available(&self) -> bool {
        self.clean_selection().is_some()
    }
}
```

Visibility matches existing siblings (`is_rust_at_path`, `is_deleted`, `clean_selection` are all `pub(super)` at `project_list.rs`). Each per-pane `enabled(*::Clean, app)` body calls `app.project_list.clean_available()`. Four trivial bodies, all delegating to the predicate's natural home — parallel to existing accessors.

If a future shared predicate appears across multiple panes, the same rule applies: name it on the data type that owns the underlying state, then panes call it.

## What dissolves

- `App::enter_action` — replaced by per-pane `bar_label(Action::Activate, app)` resolution.
- `shortcuts::enter()` const fn — replaced by `km.<scope>.display_key_for(<Scope>Action::Activate)`.
- The hardcoded `"Enter"` literal in action-bound contexts.
- `detail_groups`, `ci_groups`, `lints_groups`, `project_list_groups` — collapse into per-pane `BarRenderer` impls.
- The threaded `enter_action` / `is_rust` / `clear_lint_action` / `terminal_command_configured` / `selected_project_is_deleted` parameters across `for_status_bar` and its children. Each becomes either a per-impl convention method (`enabled`) or a data-side accessor (`clean_available`).
- The duplicate `InputContext` match. Action-bound contexts collapse to one path through `bar_renderer()`; static-only arms remain.
- `app/query/post_selection.rs::enter_action`. After this and the `sync_selected_project` rehoming, `app/query/` is gone.
- The dead `if let Some(action) = enter_action` arm inside `project_list_groups` — `enter_action` is always `None` for `InputContext::ProjectList` (verified by reading every match arm in `enter_action`). The arm has never fired.
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap` — drop in PR3 cleanup.

## The CiRuns "fetch" label is a bug

`enter_action`'s `CiRuns` branch returns `Some("fetch")` at end-of-list, but the dispatcher does nothing at EOL. Fix:

- `Ci`'s `bar_label(CiRunsAction::Activate, ...)` returns `Some("open")` when the cursor is on an actual run; `None` otherwise.
- Promoting Activate-at-EOL to FetchMore is a dispatcher change in `panes/actions.rs:handle_ci_enter` — not part of this refactor. Add a `// TODO` and move on.

## Concrete refactor sequence — three PRs

**PR 1 — additive.** Lands alone; nothing breaks.
- Create `src/tui/app/bar_renderer.rs`. Declare it in `src/tui/app/mod.rs` (`mod bar_renderer;`, private to `app/`).
- In `app/bar_renderer.rs`: define `pub(crate) trait BarRenderer`. Define `pub(crate) struct GlobalActions;` with its `RENDER_ORDER` slice and `BarRenderer` impl. Define an `impl App` block holding `pub(super) fn bar_renderer_for_focus(&self) -> Option<&dyn BarRenderer>` (the `match focus.base()` accessor).
- Implement `BarRenderer` for the six pane host structs alongside their existing `Pane` impls: `panes::PackagePane`, `panes::GitPane`, `panes::TargetsPane`, `Ci` (`src/tui/ci_state.rs`), `Lint` (`src/tui/lint_state.rs`), `ProjectList` (`src/tui/project_list.rs`).
- Add `ProjectList::clean_available()` accessor in `src/tui/project_list.rs` (replaces every per-pane `is_rust` thread of the same predicate).
- No change to the `Pane` trait at `pane/dispatch.rs:38`.

**PR 2 — the swap.**
- Hoist `make_app` from `src/tui/app/tests/mod.rs:115` (or `src/tui/interaction.rs:350`) to a shared `src/tui/test_support.rs` module gated on `#[cfg(test)]`, with `pub(super)` visibility. The module is declared in `src/tui/mod.rs` as `#[cfg(test)] mod test_support;` (no `pub` needed — `pub(super)` items inside reach every `tui::*::tests` sibling module). `shortcuts.rs::tests` imports via `use super::test_support::make_app;`. (Can't be reused as-is — both existing definitions are module-private, neither reachable from `shortcuts.rs::tests`.)
- Rewrite `for_status_bar` body. Action-bound contexts dispatch through `app.bar_renderer_for_focus()`; if `Some`, call `render_into` to push into `nav` / `actions`. Static-only arms stay inline. Globals: external context gate (`if !context.is_overlay() && !context.is_text_input()`), then `GlobalActions.render_into(app, km, …)` pushes into `globals`.
- Delete `App::enter_action` and its single call site at `render.rs:538`.
- Delete `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups`. The dead `enter_action` arm in `project_list_groups` goes with them.

**PR 3 — cleanup.**
- Delete `shortcuts::enter()` const fn.
- Confirm no literal `"Enter"` strings remain in action-bound rows.
- Collapse `for_status_bar`'s signature to `pub(super) fn for_status_bar(app: &App) -> StatusBarGroups`.
- Drop the bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`.
- Rewrite tests at `src/tui/shortcuts.rs:332+` to use the hoisted `make_app` fixture.
- Add regression test: rebinding `<Scope>Action::Activate` changes the displayed key in the bar.
- Add regression test: globals render order matches `GlobalActions::RENDER_ORDER` (locked-in test today is at `shortcuts.rs:352-355` checking `["find", "editor", "terminal", "settings"]`).

## Risks and unknowns

- **Allocation.** `display_key_for -> String` (`keymap.rs:343`). All action-bound rows go through `Cow::Owned`. Per-frame allocation across ~10 shortcuts is negligible; named so it doesn't surprise in profiles.
- **Navigation hints.** `↑/↓ nav`, `←/→ expand`, `Tab pane`, `+/- all` are not actions — informational labels. Each per-pane `BarRenderer` impl pushes them into `nav` from per-pane `Shortcut::fixed(...)` constants in `shortcuts.rs`. They don't go through the keymap.
- **Re-binding mid-frame.** `app.keymap.current()` reads once per frame; no mid-frame mutation hazard.
- **`focus.base()` vs `is_overlay()` asymmetry.** `PaneId::is_overlay()` (`spec.rs:23`) excludes `Keymap`; `InputContext::is_overlay()` (`shortcuts.rs:46`) includes it. The bar dispatches via `InputContext` for the static-only arms, sidestepping the asymmetry. Document but don't fix here.
- **Render order vs `<Action>::ALL`.** Per-pane scopes today happen to render in declaration order (verified by reading `*_groups` against the enum definitions). `GlobalAction` is the only scope where order disagrees, hence `GlobalActions::RENDER_ORDER`. Keep the convention: any per-pane impl whose render order disagrees with `<Self::Action>::ALL` declares its own ordered slice. PR1 doesn't need any per-pane override — they all match today.
- **Trait grants write access to every vec.** `render_into` takes `&mut Vec<Shortcut>` for `nav`, `actions`, and `globals`. Nothing prevents an impl from accidentally pushing into the wrong vec (e.g. `PackagePane` writing to `globals`). Convention: each impl pushes only into vecs it owns content for. Enforced at PR review, not by the trait. Six impls reviewed in one PR; the surface is small enough that this is a non-issue in practice.
- **`unreachable!()` in `GlobalActions::bar_label`.** The `Dismiss` / `NextPane` / `PrevPane` arms panic if reached, but they're only reached if iteration accidentally uses `GlobalAction::ALL` instead of `RENDER_ORDER`. PR2 includes the regression test that locks the `RENDER_ORDER` ordering; future iteration mistakes are caught there. A `debug_assert!(false, …)` plus empty-string return is an alternative; either is fine.

## Non-goals

- Not changing the action enum definitions in `keymap.rs`.
- Not changing input dispatch (key-to-action routing). EOL-Activate-as-FetchMore promotion is a separate question.
- Not touching the keymap-overlay UI (the help screen). It already reads from the keymap correctly.
- Not reconciling `PaneId::is_overlay()` with `InputContext::is_overlay()`. Out of this work.

## Definition of done

- `App::enter_action` deleted; no replacement on `App`.
- `shortcuts::enter` const fn deleted.
- No literal `"Enter"` string in `src/tui/shortcuts.rs` for action-bound rows.
- `for_status_bar` signature is `pub(super) fn for_status_bar(app: &App) -> StatusBarGroups`.
- The four per-context helper fns (`detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups`) are deleted; their logic lives in per-pane `BarRenderer` impls.
- `BarRenderer` trait exists in `src/tui/app/bar_renderer.rs` with `pub(crate)` visibility; six host structs implement it: `panes::PackagePane`, `panes::GitPane`, `panes::TargetsPane`, `Ci`, `Lint`, `ProjectList`.
- `GlobalActions` struct (with `RENDER_ORDER` slice) and `App::bar_renderer_for_focus(&self)` both live in `src/tui/app/bar_renderer.rs`.
- `Lang`, `Cpu`, `Output` paneIds fall through to `None` in `bar_renderer_for_focus`.
- `ProjectList::clean_available()` accessor exists; every per-pane `enabled(*::Clean, app)` calls it. No `clean_enabled` free fn anywhere.
- `GlobalActions: BarRenderer` exists with explicit `RENDER_ORDER` slice; `terminal_command_configured` / `selected_project_is_deleted` gating lives inside its `enabled` body, delegating to existing accessors. No `global_action_state` free fn.
- Globals' context-gating is external (caller-side: `if !context.is_overlay() && !context.is_text_input()`).
- The CiRuns `Some("fetch")` label is gone; `Ci::bar_label(Activate, ...)` returns `None` at EOL.
- The dead `enter_action` arm inside `project_list_groups` is gone with the rest of the function.
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap` is removed.
- `make_app` is hoisted to `src/tui/test_support.rs` (or equivalent shared module) and reachable from `shortcuts.rs::tests`.
- All existing bar tests pass after rewrite.
- New regression test: rebinding `<Scope>Action::Activate` changes the displayed key in the bar.
- New regression test: globals render order matches `GlobalActions::RENDER_ORDER`.
- `app/query/post_selection.rs` is one method shorter (only `sync_selected_project` remains).
