# `PaneId` ownership and `Ctx` integration contract

Companion to `docs/tui-pane-lib.md`. Locks down the two boundaries left
underspecified there:

1. Where `PaneId` lives after `tui_pane` becomes a workspace member.
2. What contract `Ctx` must satisfy so the framework can dispatch into
   it without taking re-entrant borrows.

The active plan parameterizes the framework on a single `Ctx` type the
binary substitutes (`type Ctx = crate::tui::app::App;`). This doc fixes
the remaining unknowns and rewrites every plan call site that touches
them.

---

## 1. `PaneId` design decision

**Pick: option (d) — split enum, unifying type in the binary.** The
framework owns identifiers for its three framework-internal panes; the
binary owns identifiers for its app panes; a binary-side enum unifies
the two for code that needs to talk about either kind.

### Why not the alternatives

- (a) **Framework-only `PaneId` for its three panes, separate binary
  enum.** Forces `Framework::editor_target_path` /
  `Framework::focused_pane_input_mode` to take *two* params (a
  framework id and an opaque app id), and leaves `GlobalAction::
  NextPane` unable to cycle across the union. Doesn't match the plan's
  `match focus { … }` rendering site.
- (b) **Generic associated `type PaneId`.** Adds a generic parameter to
  every framework-public function and makes
  `Framework::input_mode_queries: HashMap<PaneId, …>` impossible
  without `TypeId` keying or a per-`Ctx` map. The plan uses `PaneId`
  as a HashMap key; that requires a concrete type.
- (c) **`PaneId` stays in the binary, framework reaches it via `Ctx`.**
  The framework needs `GlobalAction::OpenKeymap` to know *which
  pane id* maps to its KeymapPane. With `PaneId` opaque to the
  framework, that requires a callback per framework pane to query
  "which app id is this framework pane?" — net loss of clarity.
- A 5th option (`TypeId`-keyed everywhere) defers the modeling to
  runtime reflection; rejected — the framework should use a real enum
  for its three panes.

### Concrete type definitions

```rust
// tui_pane/src/framework.rs (or a sibling pane_id.rs)
/// Identifies one of the framework-internal panes.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FrameworkPaneId {
    Keymap,
    Settings,
    Toasts,
}

impl FrameworkPaneId {
    pub const ALL: &'static [Self] = &[Self::Keymap, Self::Settings, Self::Toasts];
}
```

```rust
// src/tui/panes/spec.rs (binary)
/// Identifies an app-defined pane.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub enum AppPaneId {
    #[default]
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
    Finder,
}
```

```rust
// src/tui/panes/spec.rs (binary) — unifying enum the binary uses
// internally for focus, layout, behavior dispatch, and PaneId in the
// existing sense.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub enum PaneId {
    #[default]
    App(AppPaneId),
    Framework(FrameworkPaneId),
}

impl PaneId {
    pub const fn project_list() -> Self { Self::App(AppPaneId::ProjectList) }
    pub const fn keymap() -> Self { Self::Framework(FrameworkPaneId::Keymap) }
    pub const fn settings() -> Self { Self::Framework(FrameworkPaneId::Settings) }
    pub const fn toasts() -> Self { Self::Framework(FrameworkPaneId::Toasts) }
    pub const fn finder() -> Self { Self::App(AppPaneId::Finder) }

    pub const fn is_overlay(self) -> bool {
        matches!(
            self,
            Self::App(AppPaneId::Finder)
                | Self::Framework(FrameworkPaneId::Settings)
                | Self::Framework(FrameworkPaneId::Keymap),
        )
    }

    pub const fn as_framework(self) -> Option<FrameworkPaneId> {
        match self {
            Self::Framework(id) => Some(id),
            Self::App(_) => None,
        }
    }

    pub const fn as_app(self) -> Option<AppPaneId> {
        match self {
            Self::App(id) => Some(id),
            Self::Framework(_) => None,
        }
    }
}
```

The framework never names the binary's `PaneId` or `AppPaneId`. Where
it needs to talk about its own three panes, it uses
`FrameworkPaneId`. Where the binary calls into the framework with
"the focused pane is one of yours", it passes a
`FrameworkPaneId`. Where the binary asks "look up the app pane's
input mode", it passes an `AppPaneId`.

### Plan call site rewrites

Every place in `docs/tui-pane-lib.md` that currently takes `PaneId`:

| Plan site | Today's signature | Rewritten signature |
|---|---|---|
| `Navigation::dispatcher` (line 198) | `fn(Self::Variant, focused: PaneId, &mut Ctx)` | `fn(Self::Variant, focused: FrameworkPaneId \| AppPaneId, &mut Ctx)` — see note below |
| `Framework::editor_target_path` (line 946) | `fn(&self, focus: PaneId) -> Option<&Path>` | `fn(&self, focus: FrameworkPaneId) -> Option<&Path>` |
| `Framework::focused_pane_input_mode` (line 954) | `fn(&self, focus: PaneId, ctx: &Ctx) -> InputMode` | `fn(&self, ctx: &Ctx) -> InputMode` — reads `self.focused()` internally; `enum FocusedPane { App(AppPaneId), Framework(FrameworkPaneId) }` is defined in `tui_pane` |
| `Framework::input_mode_queries` (line 942) | `HashMap<PaneId, fn(&Ctx) -> InputMode>` | `HashMap<AppPaneId, fn(&Ctx) -> InputMode>` |
| `GlobalAction::OpenKeymap` (line 495) | "focus framework's KeymapPane overlay" — implicit `PaneId::Keymap` | dispatch sets focus to `FrameworkPaneId::Keymap`; binding to a binary `PaneId` happens in the `set_focus` adapter (see §3) |

To avoid leaking two types into trait signatures the framework defines
one shared enum:

```rust
// tui_pane/src/framework.rs
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FocusedPane<AppPaneId> {
    App(AppPaneId),
    Framework(FrameworkPaneId),
}
```

`FocusedPane` is generic over the binary's `AppPaneId` so the framework
never names it directly. The binary passes
`FocusedPane<crate::tui::panes::AppPaneId>` at every call site. This
preserves the "framework knows about its three panes only" rule while
giving framework signatures one parameter, not two.

`Navigation::dispatcher` (line 198) becomes:

```rust
fn dispatcher() -> fn(Self::Variant, focused: FocusedPane<AppPaneId>, ctx: &mut Ctx);
```

`AppPaneId` is an associated type on the `Ctx` trait — see §2.

The bar render dispatch site (lines 858–877 of the plan) becomes:

```rust
fn render_status_bar(app: &App, frame: &mut Frame) {
    let bar = match app.focus.current() {
        PaneId::App(AppPaneId::Package)     => bar::render(&app.panes.package,      app, &app.keymap),
        PaneId::App(AppPaneId::Git)         => bar::render(&app.panes.git,          app, &app.keymap),
        PaneId::App(AppPaneId::ProjectList) => bar::render(&app.panes.project_list, app, &app.keymap),
        PaneId::App(AppPaneId::CiRuns)      => bar::render(&app.ci,                 app, &app.keymap),
        PaneId::App(AppPaneId::Lints)       => bar::render(&app.lint,               app, &app.keymap),
        PaneId::App(AppPaneId::Targets)     => bar::render(&app.panes.targets,      app, &app.keymap),
        PaneId::App(AppPaneId::Output)      => bar::render(&app.panes.output,       app, &app.keymap),
        PaneId::App(AppPaneId::Lang)        => bar::render(&app.panes.lang,         app, &app.keymap),
        PaneId::App(AppPaneId::Cpu)         => bar::render(&app.panes.cpu,          app, &app.keymap),
        PaneId::App(AppPaneId::Finder)      => bar::render(&app.panes.finder,       app, &app.keymap),
        PaneId::Framework(id)               => bar::render_framework(id, &app.framework, app, &app.keymap),
    };
    bar.draw(frame, /* …area… */);
}
```

`bar::render_framework(id: FrameworkPaneId, &Framework<App>, &App, &Keymap<App>)`
is a single framework helper that internally dispatches over its three
panes — the binary doesn't repeat the framework-pane match.

---

## 2. The minimum `Ctx` contract

The framework needs two things from `Ctx`:

1. The associated `AppPaneId` enum so signatures can be generic without
   the binary plumbing it through every call.
2. A way to reach the `Framework<Ctx>` field. Focus is read and written
   through that field — see §3.

A trait carries this. Today the plan implies `Ctx: 'static` only; that
is not enough — the framework has nowhere to find `app.framework`.
Pure-structural (free-fn at builder time) works for some of these but
not for the associated `AppPaneId` type. Use a trait.

### `AppContext` trait

```rust
// tui_pane/src/app_context.rs
pub trait AppContext: 'static {
    /// The binary's enum of app-defined pane identifiers. Distinct
    /// from `FrameworkPaneId`.
    type AppPaneId: Copy + Eq + Hash + std::fmt::Debug + 'static;

    /// Mutable access to the framework aggregator the binary owns.
    /// Framework dispatchers call this from contexts where they hold
    /// `&mut Ctx` to mutate framework-pane state.
    fn framework_mut(&mut self) -> &mut Framework<Self>;

    /// Shared access to the same field. Bar rendering needs
    /// `&Framework<Ctx>` and `&Ctx` simultaneously, so the binary's
    /// implementation must return a borrow that does not conflict
    /// with other shared reads — a plain field reference works.
    fn framework(&self) -> &Framework<Self>;

    /// Request a focus change. Framework dispatchers call this for
    /// `GlobalAction::{NextPane, PrevPane, OpenKeymap,
    /// OpenSettings, Dismiss}`. The binary's implementation routes
    /// through its `Focus` subsystem (overlay-return memory, visited
    /// set, `pane_state` lookup) which in turn writes
    /// `self.framework.set_focused(...)`.
    ///
    /// Invariant: framework code never calls `framework.set_focused`
    /// directly — every focus transition originating in the framework
    /// goes through this method so the binary's `Focus` policy fires.
    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>);
}
```

Focus reads happen on `Framework<Ctx>` (`framework.focused()`); focus
writes go through `AppContext::set_focus`. Reads stay on the framework
side so framework code (bar rendering, dispatch routing) never calls
back through `Ctx` to learn what's focused. Writes go through the trait
so the binary's `Focus` subsystem (richer policy) is the single point
that updates the framework's `focused` field.

Wherever the plan writes `Ctx`, it now writes `Ctx: AppContext`. The
trait carries the associated `AppPaneId`, so traits like `Navigation`
become:

```rust
pub trait Navigation<Ctx: AppContext> {
    type Variant: Action + 'static;
    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Variant;
    const DOWN:  Self::Variant;
    const LEFT:  Self::Variant;
    const RIGHT: Self::Variant;
    fn defaults() -> Bindings<Self::Variant>;
    fn dispatcher() -> fn(Self::Variant, focused: FocusedPane<Ctx::AppPaneId>, ctx: &mut Ctx);
}
```

`Shortcuts<Ctx>` and `Globals<Ctx>` likewise pick up the bound. The
trait does *not* require that `Ctx` expose pane state — every pane's
own state is reached via the per-pane dispatcher's free fn navigating
through `Ctx` (`&mut ctx.panes.package`, etc.). The trait only carries
the cross-cutting plumbing the framework genuinely needs.

---

## 3. Focus tracking

**Reads on the framework, writes through the trait.**

`Framework<Ctx>` carries `focused: FocusedPane<Ctx::AppPaneId>` and
exposes `focused(&self) -> FocusedPane<Ctx::AppPaneId>` plus
`set_focused(&mut self, FocusedPane<Ctx::AppPaneId>)`. The trait
surfaces a single write method, `AppContext::set_focus`, which framework
dispatchers call for every focus transition.

Why this split:

- The dependency goes binary → framework for reads. Framework code
  that needs to know what's focused (bar rendering, dispatch routing)
  reads `self.focused` directly; it never calls back through `Ctx`.
- Writes route through the binary so cargo-port's `Focus` subsystem
  (`src/tui/focus.rs` — overlay-return memory, visited tracking,
  `pane_state` lookup) is the single place focus transitions land.
  Pulling that policy into the framework would force every consumer
  to inherit cargo-port-specific overlay rules; making the framework
  write `set_focused` directly bypasses the policy.
- Invariant: only the binary's `Focus` adapter calls
  `framework.set_focused`. Every framework-originated transition goes
  through `ctx.set_focus`.

Concretely the binary implements:

```rust
// src/tui/app/mod.rs
impl AppContext for App {
    type AppPaneId = crate::tui::panes::AppPaneId;

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        // Route through the existing Focus subsystem so visited /
        // pane_state / overlay-return bookkeeping fires on every
        // transition. Focus::set then writes self.framework.set_focused.
        let pane: PaneId = focus.into();   // FocusedPane → PaneId
        if pane.is_overlay() {
            self.focus.open_overlay(&mut self.framework, pane);
        } else {
            self.focus.set(&mut self.framework, pane);
        }
    }
}
```

The binary's `Focus` subsystem owns the actual `framework.set_focused`
call:

```rust
// src/tui/focus.rs
impl Focus {
    pub fn set(&mut self, framework: &mut Framework<App>, pane: PaneId) {
        // ...existing visited / pane_state bookkeeping...
        framework.set_focused(pane.into());
    }
    pub fn open_overlay(&mut self, framework: &mut Framework<App>, pane: PaneId) { /* ... */ }
}
```

`PaneId` and `FocusedPane` are isomorphic by construction — the binary
defined `PaneId` as a thin wrapper over the framework's `FocusedPane`
value (App / Framework split). The `From` adapters drop out at compile
time.

**Sync invariant:** `app.focus.current()` and `app.framework.focused()`
always agree. `Focus::set` and `Focus::open_overlay` write
`framework.set_focused` synchronously as part of their existing
bookkeeping; no other code path mutates either side. Reads can use
whichever is convenient at the call site (the binary tends to use
`focus.current()` for its `PaneId` flavor; framework code reads
`self.focused`).

Call sites:

- `Framework::focused_pane_input_mode` — reads `self.focused()`
  internally; callers write `framework.focused_pane_input_mode(ctx)`.
- `GlobalAction::Dismiss` dispatch — reads
  `ctx.framework().focused()` to compute the dismiss target, then
  calls `ctx.set_focus(target)`.
- `GlobalAction::NextPane` / `PrevPane` — read
  `ctx.framework().focused()`, compute next, call
  `ctx.set_focus(next)`.
- `GlobalAction::OpenKeymap` / `OpenSettings` — call
  `ctx.set_focus(FocusedPane::Framework(FrameworkPaneId::Keymap))` /
  `…::Settings` directly.

---

## 4. `App.framework` field

**Place at the top level: `App.framework: Framework<App>`.**

```rust
pub(super) struct App {
    pub(super) net:               Net,
    pub(super) panes:             Panes,
    pub(super) background:        Background,
    pub(super) inflight:          Inflight,
    pub(super) lint:              Lint,
    pub(super) ci:                Ci,
    pub(super) config:            Config,
    pub(super) keymap:            Keymap<App>,           // see §5
    pub(super) project_list:      ProjectList,
    pub(super) scan:              Scan,
    pub(super) startup:           Startup,
    pub(super) focus:             Focus,
    pub(super) overlays:          Overlays,
    /// Framework aggregator: KeymapPane, SettingsPane, Toasts plus
    /// the per-app input-mode query registry. Populated by the
    /// `KeymapBuilder` at startup.
    pub(super) framework:         Framework<App>,
    /* …unchanged remainder… */
}
```

Reasons against nesting (`App.tui.framework`):

- Cargo-port's existing layout has every subsystem at the top level
  (`net`, `panes`, `inflight`, `lint`, `ci`, `config`, `keymap`,
  `focus`, `overlays`). A nested `tui` field would invent a new tier
  for one item.
- The framework is peer to `panes` and `keymap` — same conceptual
  level, same lifetime, same access pattern.

`AppContext::framework_mut` returns `&mut self.framework`. Because the
field sits next to (not inside) `panes`, splitting borrows like
`framework.dispatch(action, &mut ctx.panes.package)` only requires
disjoint-field reasoning that the borrow checker already supports.

---

## 5. cargo-port `App` changes

Net diff against today's `App` struct:

### Additions

| Field | Type | Purpose |
|---|---|---|
| `framework` | `Framework<App>` | Aggregates `KeymapPane`, `SettingsPane`, `Toasts`, plus the `input_mode_queries` registry. Replaces ad-hoc overlay state today scattered across `Overlays` (see "Removals/changes" below). |
| `keymap` (re-typed) | `Keymap<App>` from `tui_pane` (replacing today's `Keymap` in `crate::tui::keymap_state`) | Built once at startup via `Keymap::<App>::builder(quit, restart, dismiss).register::<…>().with_settings(…).with_navigation::<AppNavigation>().with_globals::<AppGlobals>().load_toml(…).build()`. The existing `WatchedFile<ResolvedKeymap>` reload machinery lives next to it (or composes it — see "Open issue" below). |

### Removals from `App`

| Item | Today | After refactor |
|---|---|---|
| `App::input_context()` (line 722) | Returns the `InputContext` enum tag based on `focus` + overlay flags | **Deleted.** Bar render and input router consult `app.focus.current()` directly per §3. |
| `App::enter_action(...)` family | Per-pane label resolution for the bar | **Deleted.** Each pane's `Shortcuts::label(action, ctx)` impl returns the label (defaults to `Some(action.bar_label())`; pane overrides only for state-dependent labels). |
| `shortcuts::InputContext` enum | App-side enum for routing | **Deleted.** Use `PaneId::App(AppPaneId::…)` / `PaneId::Framework(FrameworkPaneId::…)` everywhere. |

### Free-fn dispatchers

Free fns sit *next to* the thing they dispatch on, not on `App`:

```rust
// tui_pane re-export point isn't relevant — these live in the binary
// because they touch App-specific subsystems.

// src/tui/app/lifecycle.rs (new, or fold into existing async_tasks/dismiss)
pub(crate) fn quit(app: &mut App)    { app.set_quit() }
pub(crate) fn restart(app: &mut App) { app.set_restart() }
pub(crate) fn dismiss(app: &mut App) {
    let target = app.focused_dismiss_target();
    app.dismiss(target);
}
```

```rust
// src/tui/panes/package.rs
pub(crate) fn dispatch_package(action: PackageAction, app: &mut App) {
    let pane = &mut app.panes.package;
    match action {
        PackageAction::Activate => { /* … */ },
        PackageAction::Clean    => { /* … */ },
    }
}

impl Shortcuts<App> for PackagePane {
    type Variant = PackageAction;
    const SCOPE_NAME: &'static str = "package";
    fn defaults() -> Bindings<PackageAction> { /* … */ }
    fn label(&self, action: PackageAction, ctx: &App) -> Option<&'static str> { /* … */ }
    fn state(&self, action: PackageAction, ctx: &App) -> ShortcutState { /* … */ }
    fn dispatcher() -> fn(PackageAction, &mut App) { dispatch_package }
}
```

The same pattern repeats in `panes/git.rs`, `panes/project_list.rs`,
`panes/ci.rs`, `panes/lints.rs`, `panes/targets.rs`, `panes/output.rs`,
`panes/lang.rs`, `panes/cpu.rs`, and `finder.rs`.

`AppNavigation` and `AppGlobalAction` get their own free-fn
dispatchers at the same module level:

```rust
// src/tui/keymap/navigation.rs (new file in the binary)
pub(crate) fn dispatch_navigation(
    action: NavigationAction,
    focused: FocusedPane<AppPaneId>,
    app: &mut App,
) { /* per-action match, routes to the focused pane's scrollable */ }
```

### Methods that *stay* on `App` but adapt

| Method | Adaptation |
|---|---|
| `App::focused_dismiss_target` | Unchanged signature; `dismiss(app)` free fn calls it via the two-step bind shown in plan line 510. |
| `App::set_quit` / `App::set_restart` | Unchanged; the new free fns wrap them. |
| `App::sync_selected_project` | Unchanged. |
| `App::prune_toasts` | Unchanged in body, but `app.framework.toasts` may take over what `app.toasts` does today — see "Open issue" below. |

### Open issue: `app.toasts` vs `app.framework.toasts`

Today `App` owns `toasts: ToastManager`. The framework defines a
`Toasts<Ctx>` pane inside `Framework<Ctx>`. Two options for the migration:

- (i) `Framework<App>::toasts` *is* the existing `ToastManager`
  (rename + relocate); `app.toasts` field is removed and every
  `app.toasts.push_*` call becomes `app.framework.toasts.push_*`.
  Largest churn, cleanest end state.
- (ii) `Framework<App>::toasts` is a thin shim that holds
  `ToastsAction` defaults + bar info, while `ToastManager` stays at
  `app.toasts`. Smaller churn, two places that talk about toasts.

Option (i) is the right destination; flag this for the migration plan
as an explicit Phase-7 (or Phase-9 alongside `handle_toast_key`)
decision rather than picking it silently here.

---

## 6. Migration order

Map each App-side change to one of the plan's phases (1–10). Phases
1–4 are framework-only and don't touch `App`. Phases 5+ touch the
binary.

| Plan phase | App change |
|---|---|
| **3** | Framework defines `tui_pane::PaneId`, `FocusedPane`, `AppContext` trait, `Framework<Ctx>` aggregator. No App impact. |
| **5** | Binary defines `AppPaneId`, refactors `PaneId` to the wrapping enum from §1, adds new action enums (`NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction`). Adds `ExpandRow`/`CollapseRow` to `ProjectListAction`. Writes `impl Shortcuts<App>` for each app pane and `impl AppContext for App` — `App` gains the `framework: Framework<App>` field and the new `keymap: Keymap<App>` field. The binary's `Focus` subsystem writes `framework.set_focused(...)` as part of its existing transitions. The `Keymap::<App>::builder(quit, restart, dismiss)…build()` chain runs at startup. Old `App::enter_action` and old `for_status_bar` *both still exist* per the plan; new code paths are populated but not consumed yet. |
| **6** | Overlay handlers (`handle_finder_key`, `handle_settings_key`, `handle_keymap_key`) reroute through scope dispatch. The old `app.overlays.is_*_open()` flags can collapse onto `matches!(app.focus.current(), PaneId::Framework(_))`. |
| **7** | Base-pane navigation handlers consult `NavigationAction`. `App` unchanged. |
| **8** | Toasts/Output/structural-Esc rerouting. The Open Issue above (toasts ownership) resolves here: `ToastManager` either stays on `App` or moves into `Framework<App>::toasts`. |
| **9** | Delete `App::enter_action`, `App::input_context`, `shortcuts::InputContext`, the seven `Shortcut::fixed` constants, the four group helpers. The `keymap: Keymap<App>` field replaces the old `keymap: crate::tui::keymap_state::Keymap` — the `WatchedFile`-based reload machinery either composes the new type or the binary keeps both fields side-by-side until the reload path is rewritten. |
| **10** | Regression tests only — no `App` changes. |

---

## 7. `InputMode` query plumbing

Each app pane registers a free fn `fn(&App) -> InputMode` at builder
time. Concrete table:

```rust
// src/tui/panes/finder.rs
pub(crate) fn finder_input_mode(app: &App) -> InputMode {
    if app.focus.current() == PaneId::App(AppPaneId::Finder) {
        InputMode::TextInput
    } else {
        InputMode::Navigable
    }
}

// src/tui/panes/output.rs
pub(crate) fn output_input_mode(_app: &App) -> InputMode { InputMode::Static }

// src/tui/panes/project_list.rs
pub(crate) fn project_list_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/package.rs
pub(crate) fn package_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/git.rs
pub(crate) fn git_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/lang.rs
pub(crate) fn lang_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/targets.rs
pub(crate) fn targets_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/lints.rs
pub(crate) fn lints_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/ci.rs
pub(crate) fn ci_runs_input_mode(_app: &App) -> InputMode { InputMode::Navigable }

// src/tui/panes/cpu.rs
pub(crate) fn cpu_input_mode(_app: &App) -> InputMode { InputMode::Static }
```

Routing confirmation, every pane:

| `AppPaneId` | Free fn | Result |
|---|---|---|
| `ProjectList` | `project_list_input_mode` | `Navigable` |
| `Package` | `package_input_mode` | `Navigable` |
| `Lang` | `lang_input_mode` | `Navigable` |
| `Cpu` | `cpu_input_mode` | `Static` |
| `Git` | `git_input_mode` | `Navigable` |
| `Targets` | `targets_input_mode` | `Navigable` |
| `Lints` | `lints_input_mode` | `Navigable` |
| `CiRuns` | `ci_runs_input_mode` | `Navigable` |
| `Output` | `output_input_mode` | `Static` |
| `Finder` | `finder_input_mode` | `TextInput` when focused, else `Navigable` |

`KeymapBuilder::register::<P>()` reads `P::input_mode()` (the trait
method on `Shortcuts<Ctx>` returning `fn(&Ctx) -> InputMode`) plus
`P::APP_PANE_ID` (an associated const of `AppPaneId` value) and inserts
into `Framework::input_mode_queries`. Sketch:

```rust
// On each Shortcuts<App> impl:
impl Shortcuts<App> for FinderPane {
    /* … */
    fn input_mode() -> fn(&App) -> InputMode { finder_input_mode }
    const APP_PANE_ID: AppPaneId = AppPaneId::Finder;
}
```

Framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) are special-
cased inside `Framework::focused_pane_input_mode` (they read their own
internal `Mode` flag — plan lines 957–958). They do not appear in
`input_mode_queries`.

The `TextInput`-when-focused pattern for `FinderPane` is the only
focus-conditional case among app panes today. Lifting it into a free
fn (rather than reading `pane.is_visible()` from inside the function)
keeps the registration symmetric with the other nine panes — the
trait always returns a `fn(&App) -> InputMode` pointer, never a method
on `&self`.

---

## Summary of new types in `tui_pane`

```rust
pub enum FrameworkPaneId { Keymap, Settings, Toasts }
pub enum FocusedPane<AppPaneId> { App(AppPaneId), Framework(FrameworkPaneId) }

pub trait AppContext: 'static {
    type AppPaneId: Copy + Eq + Hash + std::fmt::Debug + 'static;
    fn framework(&self)        -> &Framework<Self>;
    fn framework_mut(&mut self) -> &mut Framework<Self>;
}

// Focus lives on Framework<Ctx>, not the trait:
impl<Ctx: AppContext> Framework<Ctx> {
    pub fn focused(&self) -> FocusedPane<Ctx::AppPaneId> { /* ... */ }
    pub fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) { /* ... */ }
}
```

## Summary of new/changed types in the binary

```rust
// src/tui/panes/spec.rs
pub enum AppPaneId { ProjectList, Package, Lang, Cpu, Git, Targets, Lints, CiRuns, Output, Finder }
pub enum PaneId    { App(AppPaneId), Framework(FrameworkPaneId) }

// src/tui/app/mod.rs — App gains:
pub(super) framework: Framework<App>,
pub(super) keymap:    Keymap<App>,        // re-typed from today's local Keymap

// src/tui/app/mod.rs — App loses:
//   App::input_context (line 722)
//   App::enter_action (and family)

// src/tui/shortcuts.rs — InputContext enum: deleted entirely.
```
