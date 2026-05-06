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
            let state = if self.enabled(action, app) { ShortcutState::Enabled } else { ShortcutState::Disabled };
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

`bar_label` returns `None` to skip a row, `Some("…")` to render with that label. The match-on-action keeps the per-action logic together; `enabled` and `activate_label` stay private to the impl. Every per-pane impl follows the same internal layout.

### `bar_label` / `enabled` for each pane

Sketches below show what each `impl` is responsible for. The exact source of each match arm is the existing code as cited.

**`PackagePane`** (logic ported from `enter_action`'s `DetailFields` Package branch + `detail_groups`):

```rust
fn bar_label(&self, action: PackageAction, app: &App) -> Option<&'static str> {
    match action {
        PackageAction::Activate => self.activate_label(app),
        PackageAction::Clean    => Some("clean"),
    }
}

fn enabled(&self, action: PackageAction, app: &App) -> bool {
    match action {
        PackageAction::Clean => app.project_list.clean_available(),
        _                    => true,
    }
}

fn activate_label(&self, _app: &App) -> Option<&'static str> {
    let pkg = self.content()?;
    let fields = panes::package_fields_from_data(pkg);
    let field = *fields.get(self.viewport.pos())?;
    if field == DetailField::CratesIo && pkg.crates_version.is_some() {
        Some("open")
    } else {
        None
    }
}
```

**`GitPane`** (logic from `enter_action`'s `DetailFields` Git branch + `detail_groups`):

```rust
fn bar_label(&self, action: GitAction, _app: &App) -> Option<&'static str> {
    match action {
        GitAction::Activate => self.activate_label(),
        GitAction::Clean    => Some("clean"),
    }
}

fn enabled(&self, action: GitAction, app: &App) -> bool {
    match action {
        GitAction::Clean => app.project_list.clean_available(),
        _                => true,
    }
}

fn activate_label(&self) -> Option<&'static str> {
    let git = self.content()?;
    let pos = self.viewport.pos();
    match panes::git_row_at(git, pos) {
        Some(GitRow::Remote(remote)) if remote.full_url.is_some() => Some("open"),
        _ => None,
    }
}
```

**`TargetsPane`** (logic from `enter_action`'s `DetailTargets` arm + `detail_groups`):

```rust
fn bar_label(&self, action: TargetsAction, _app: &App) -> Option<&'static str> {
    match action {
        TargetsAction::Activate     => Some("run"),
        TargetsAction::ReleaseBuild => Some("release"),
        TargetsAction::Clean        => Some("clean"),
    }
}

fn enabled(&self, action: TargetsAction, app: &App) -> bool {
    match action {
        TargetsAction::Clean => app.project_list.clean_available(),
        _                    => true,
    }
}
```

**`Ci`** (logic from `enter_action`'s `CiRuns` arm + `ci_groups`; the EOL `Some("fetch")` case is dropped as documented in "The CiRuns 'fetch' label is a bug"):

```rust
fn bar_label(&self, action: CiRunsAction, app: &App) -> Option<&'static str> {
    match action {
        CiRunsAction::Activate   => self.activate_label(app),
        CiRunsAction::FetchMore  => Some("fetch more"),
        CiRunsAction::ToggleView => Some("branch/all"),
        CiRunsAction::ClearCache => Some("clear cache"),
    }
}

fn enabled(&self, _action: CiRunsAction, _app: &App) -> bool {
    true
}

fn activate_label(&self, app: &App) -> Option<&'static str> {
    let path = app.project_list.selected_project_path()?;
    let run_count = app
        .project_list
        .ci_info_for(path)
        .map_or(0, |info| info.runs.len());
    if self.viewport.pos() < run_count {
        Some("open")
    } else {
        None
    }
}
```

**`Lint`** (logic from `enter_action` (which has no Lints arm — Activate is always `None` today) + `lints_groups` + `render.rs:544-549`):

```rust
fn bar_label(&self, action: LintsAction, app: &App) -> Option<&'static str> {
    match action {
        LintsAction::Activate     => None,  // today's bar never shows Enter for Lints
        LintsAction::ClearHistory => self.clear_history_label(app),
    }
}

fn enabled(&self, _action: LintsAction, _app: &App) -> bool {
    true
}

fn clear_history_label(&self, app: &App) -> Option<&'static str> {
    let path = app.project_list.selected_project_path()?;
    let runs = app.lint_at_path(path)?;
    (!runs.runs().is_empty()).then_some("clear cache")
}
```

**`ProjectListPane`** (logic from `enter_action` (no ProjectList arm — Activate always `None` today) + `project_list_groups`; ExpandAll/CollapseAll are emitted as the combined nav row, not as actions):

```rust
fn bar_label(&self, action: ProjectListAction, _app: &App) -> Option<&'static str> {
    match action {
        ProjectListAction::ExpandAll | ProjectListAction::CollapseAll => None,
        ProjectListAction::Clean                                       => Some("clean"),
    }
}

fn enabled(&self, action: ProjectListAction, app: &App) -> bool {
    match action {
        ProjectListAction::Clean => app.project_list.clean_available(),
        _                         => true,
    }
}
```

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
            let state = if self.enabled(action, app) { ShortcutState::Enabled } else { ShortcutState::Disabled };
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

Rather than extend the `Pane` trait at `pane/dispatch.rs:38` (which would force adding `bar_renderer` to every pane impl plus inventing `App::focused_pane()` to traverse the scattered hosts), add a dedicated single-purpose accessor on `App` (full match listed in the pane coverage table below).

The match owns the only `PaneId -> pane host` lookup the bar needs. It's one place, named for its purpose, and doesn't bloat the existing `Pane` trait. Each of the six host structs implements `BarRenderer` directly (alongside their existing `Pane` impl).

Panes without a scope enum (`Lang`, `Cpu`, `Output`, plus all overlay/static panes) fall through to `None`; the bar emits an empty action group for them or routes to a static-only arm.

**Borrow story.** `bar_renderer_for_focus` returns `&dyn BarRenderer` borrowed from `&self`. The bar then calls `renderer.render_into(app, km, …)` with another `&App` reference. Both are shared borrows of `App`, so the double-borrow type-checks. No `&mut` paths involved.

### Pane coverage table

The "host struct" column is the actual struct that implements `BarRenderer`. Some panes' host structs are not `*Pane`-named (`Ci`/`Lint` are subsystem structs that play the pane role for `PaneId::CiRuns`/`Lints`).

| `PaneId` | Scope enum | Host struct (where `impl Pane` lives) | `BarRenderer` impl location | App field path | Nav constants pushed |
|---|---|---|---|---|---|
| `ProjectList` | `ProjectListAction` | `ProjectListPane` | `src/tui/panes/pane_impls.rs:354` | `app.panes.project_list` | `NAV`, `ARROWS_EXPAND`, paired Expand/Collapse row from keymap (replaces `EXPAND_COLLAPSE_ALL`), `TAB_PANE` |
| `Package` | `PackageAction` | `PackagePane` | `src/tui/panes/pane_impls.rs:59` | `app.panes.package` | `NAV`, `TAB_PANE` |
| `Git` | `GitAction` | `GitPane` | `src/tui/panes/pane_impls.rs:245` | `app.panes.git` | `NAV`, `TAB_PANE` |
| `Targets` | `TargetsAction` | `TargetsPane` | `src/tui/panes/pane_impls.rs:325` | `app.panes.targets` | `NAV`, `TAB_PANE` |
| `CiRuns` | `CiRunsAction` | `Ci` | `src/tui/ci_state.rs:271` | `app.ci` | `NAV`, `TAB_PANE` |
| `Lints` | `LintsAction` | `Lint` | `src/tui/lint_state.rs:214` | `app.lint` | `NAV`, `TAB_PANE` |
| `Lang` | none | `LangPane` (`pane_impls.rs:92`) | n/a — no `BarRenderer` impl | n/a | n/a |
| `Cpu` | none | `CpuPane` (`pane_impls.rs:159`) | n/a | n/a | n/a |
| `Output` | none | `OutputPane` (`pane_impls.rs:376`) | n/a | n/a | n/a |
| `Toasts` | none (uses `GlobalAction::Dismiss`) | `ToastManager` (`toasts/manager.rs:760`) | n/a — static-only arm | n/a | n/a |
| `Finder` / `Settings` / `Keymap` | none | overlay panes (`overlays/pane_impls.rs`) | n/a — static-only arms | n/a | n/a |

Each `BarRenderer` impl is added **alongside the existing `impl Pane for X`** in the same file. Nav constants are at `src/tui/shortcuts.rs:99-105`. Each `BarRenderer::render_into` body pushes its row's constants into the `nav` Vec before iterating actions.

**Data-vs-UI split for ProjectList.** `Pane` and `BarRenderer` go on `ProjectListPane` (UI struct, holds the viewport). The `clean_available()` accessor still goes on `ProjectList` (data subsystem at `src/tui/project_list.rs`). Inside `ProjectListPane`'s `bar_label(*::Clean, app)` body, the call is `app.project_list.clean_available()` — UI struct asks the data subsystem.

**`bar_renderer_for_focus` match.** The match returns `&dyn BarRenderer` from the App field listed in the table:

```rust
pub(super) fn bar_renderer_for_focus(&self) -> Option<&dyn BarRenderer> {
    match self.focus.base() {
        PaneId::Package     => Some(&self.panes.package),
        PaneId::Git         => Some(&self.panes.git),
        PaneId::Targets     => Some(&self.panes.targets),
        PaneId::ProjectList => Some(&self.panes.project_list),
        PaneId::CiRuns      => Some(&self.ci),
        PaneId::Lints       => Some(&self.lint),
        _ => None,
    }
}
```

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

## Concrete refactor sequence — four phases

Each phase is one merge. Integer numbering only; no sub-phases.

**Phase 1 — Keymap supports multiple keys per action.**

Today's `ScopeMap` at `src/keymap.rs:325-329` binds one key per action:

```rust
pub struct ScopeMap<A> {
    pub by_key:    HashMap<KeyBind, A>,
    pub by_action: HashMap<A, KeyBind>,    // ← single key per action
}
```

This blocks the goal: ExpandAll wants to bind to both `=` and `+`, CollapseAll wants `-`. The bar must show all bound keys, and dispatch must accept any of them.

- Change `by_action` to `HashMap<A, Vec<KeyBind>>`. Dispatch (`by_key`) stays 1-to-1 — multiple keys mapping to one action is well-defined; one key mapping to multiple actions is not.
- Add `pub fn display_keys_for(self: &ScopeMap<A>, action: A) -> &[KeyBind]` returning all bound keys in insertion order.
- Keep `display_key_for(action) -> String` returning the **primary** (first) bound key, for callers that only want one.
- Update keymap TOML parsing to accept either a single string (`activate = "Enter"`) or an array (`expand_all = ["=", "+"]`). Single string remains the common form; array opens multi-binding.
- Update default keymap definitions (`src/keymap.rs:default_*`) to bind `ExpandAll` to `["=", "+"]` and `CollapseAll` to `["-"]`. Confirm prior single-key defaults still parse via the back-compat single-string path.
- Tests: multi-binding round-trips through TOML; `display_keys_for` returns all bound keys; dispatch fires for every bound key.

No changes to `shortcuts.rs` or `App` in this phase — purely a keymap-side data model change with new API surface.

**Phase 2 — Additive: `BarRenderer` trait, impls, accessors.** Lands alone; nothing breaks.

- Create `src/tui/app/bar_renderer.rs`. Declare it in `src/tui/app/mod.rs` (`mod bar_renderer;`, private to `app/`).
- In `app/bar_renderer.rs`: define `pub(crate) trait BarRenderer`. Define `pub(crate) struct GlobalActions;` with its `RENDER_ORDER` slice and `BarRenderer` impl. Define an `impl App` block holding `pub(super) fn bar_renderer_for_focus(&self) -> Option<&dyn BarRenderer>` (the `match focus.base()` accessor).
- Implement `BarRenderer` for the six pane host structs alongside their existing `Pane` impls: `panes::PackagePane`, `panes::GitPane`, `panes::TargetsPane`, `Ci` (`src/tui/ci_state.rs`), `Lint` (`src/tui/lint_state.rs`), `ProjectList` (`src/tui/project_list.rs`).
- ProjectList's impl uses Phase 1's `display_keys_for` to build the combined Expand/Collapse nav row (see "ProjectList paired-action nav row" below). `bar_label(ExpandAll, _)` and `bar_label(CollapseAll, _)` return `None` — the iteration skips them; the combined row is emitted as a nav-strip shortcut instead.
- Add `ProjectList::clean_available()` accessor in `src/tui/project_list.rs` (replaces every per-pane `is_rust` thread of the same predicate).
- No change to the `Pane` trait at `pane/dispatch.rs:38`.

**Phase 3 — The swap.**

- Hoist `make_app` from `src/tui/app/tests/mod.rs:115` (or `src/tui/interaction.rs:350`) to a shared `src/tui/test_support.rs` module gated on `#[cfg(test)]`, with `pub(super)` visibility. The module is declared in `src/tui/mod.rs` as `#[cfg(test)] mod test_support;`. `shortcuts.rs::tests` imports via `use super::test_support::make_app;`. (Can't be reused as-is — both existing definitions are module-private, neither reachable from `shortcuts.rs::tests`.)
- Rewrite `for_status_bar` body. Action-bound contexts dispatch through `app.bar_renderer_for_focus()`; if `Some`, call `render_into` to push into `nav` / `actions`. Static-only arms stay inline. Globals: external context gate (`if !context.is_overlay() && !context.is_text_input()`), then `GlobalActions.render_into(app, km, …)` pushes into `globals`.
- Delete `App::enter_action` and its single call site at `render.rs:538`.
- Delete `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups`. The dead `enter_action` arm in `project_list_groups` goes with them.
- Delete the static `EXPAND_COLLAPSE_ALL: Shortcut::fixed("+/-", "all")` constant at `shortcuts.rs:105` — ProjectList's impl now sources it from the keymap.

**Phase 4 — Cleanup.**

- Delete `shortcuts::enter()` const fn.
- Confirm no literal `"Enter"` strings remain in action-bound rows.
- Collapse `for_status_bar`'s signature to `pub(super) fn for_status_bar(app: &App) -> StatusBarGroups`.
- Drop the bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`.
- Rewrite tests at `src/tui/shortcuts.rs:332+` to use the hoisted `make_app` fixture.
- Add regression test: rebinding `<Scope>Action::Activate` changes the displayed key in the bar.
- Add regression test: globals render order matches `GlobalActions::RENDER_ORDER` (locked-in test today is at `shortcuts.rs:352-355` checking `["find", "editor", "terminal", "settings"]`).
- Add regression test: rebinding `ProjectListAction::ExpandAll` to a different key list changes the combined Expand/Collapse nav row's displayed keys.

### ProjectList paired-action nav row

ExpandAll and CollapseAll render as one combined nav row (the current `+/-` slot), sourced from the keymap. ProjectList's `BarRenderer::render_into` emits this row before iterating actions:

```rust
let expand_keys   = format_action_keys(km.project_list.display_keys_for(ProjectListAction::ExpandAll));
let collapse_keys = format_action_keys(km.project_list.display_keys_for(ProjectListAction::CollapseAll));
nav.push(Shortcut::from_keymap(format!("{expand_keys}/{collapse_keys}"), "all"));
```

`format_action_keys(&[KeyBind]) -> String` joins multiple bindings for one action with `,` (e.g. `[=, +] -> "=/+"` … wait, that produces ambiguity with the slash between expand and collapse). The bar's combined row uses `,` between same-action keys and `/` between paired actions, giving e.g. `"=,+/-"` for ExpandAll=`[=,+]`, CollapseAll=`[-]`. Defined once in `app/bar_renderer.rs`:

```rust
fn format_action_keys(keys: &[KeyBind]) -> String {
    keys.iter().map(|k| k.display()).collect::<Vec<_>>().join(",")
}
```

The pair-separator `/` is hardcoded inside ProjectList's impl since it's specific to this paired display. Other panes that show a single action's keys just use `format_action_keys` directly.

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

- `ScopeMap::by_action` is `HashMap<A, Vec<KeyBind>>`; `display_keys_for(action) -> &[KeyBind]` exists; `display_key_for(action) -> String` returns the primary (first) key.
- Default keymap binds `ProjectListAction::ExpandAll` to `["=", "+"]` and `CollapseAll` to `["-"]`. TOML parsing accepts both single-string and array forms for any action.
- `App::enter_action` deleted; no replacement on `App`.
- `shortcuts::enter` const fn deleted.
- `EXPAND_COLLAPSE_ALL` static constant deleted; ProjectList's nav row sources Expand/Collapse keys from `display_keys_for`.
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
