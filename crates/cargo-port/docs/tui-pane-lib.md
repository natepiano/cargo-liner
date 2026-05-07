# `tui_pane` library + universal keymap

## Design specifications

This doc is the high-level plan + roadmap. Formal specs live in sibling files:

| Spec | Covers |
|---|---|
| `phase-01-cargo-mechanics-DONE.md` | Root + library `Cargo.toml`, workspace lints (incl. `missing_docs = "deny"` from day one), resolver, `cargo install --path .` behavior, stale-path sweep, per-phase rustdoc precondition |
| `phase-01-ci-tooling-DONE.md` | CI invocation inventory, per-invocation scope decisions, auto-memory implications, recommended commit ordering |
| `test-infra.md` | Private `test_support/` module, `cfg(test)` `TestCtx` fixture, unit-test placement, integration-test layout |
| `core-api.md` | Full public API: `KeyBind`, `Bindings<A>`, `ScopeMap<A>`, traits, `Keymap<Ctx>`, `KeymapBuilder<Ctx>`, `Framework<Ctx>`, `SettingsRegistry<Ctx>`, errors, re-exports |
| `macros.md` | `bindings!` and `action_enum!` formal grammar + expansion + hygiene |
| `paneid-ctx.md` | `tui_pane::PaneId` (framework panes) / `AppPaneId` (binary) / `FocusedPane` (wrapping), `AppContext` trait, focus tracking, `App` field changes, migration order, `Mode` query plumbing |
| `toml-keys.md` | TOML grammar, error taxonomy, vim-after-TOML worked examples, full `KeyBind::display` / `display_short` mapping tables |

The plan body below summarizes; the specs are authoritative for their topics.

## Principle

**Every key the user can press is bound through the keymap.** No `KeyCode::Enter` / `Up` / `Esc` / `Tab` / `Left` / `Right` matches in any input handler. No literal `"Enter"` / `"Esc"` / `"Tab"` / `"↑/↓"` / `"+/-"` strings in any bar code. Every shortcut row in the bar comes from a binding lookup. Rebinding any key updates the bar and the dispatcher in lockstep.

This is a single coherent refactor — partial keymap-driving (e.g. action-bound contexts only, or focused panes only) is rejected as inconsistent.

**Lens for every API decision:** the API the *client* (cargo-port) defines is as simple as possible. The framework absorbs structural complexity; the client describes what its panes do.

---

## Workspace structure

The keymap, bar, and overlay machinery live in a workspace member crate named **`tui_pane`**. Modeled structurally after the `bevy_hana` workspace.

The first development step converts `cargo-port-api-fix` into a Cargo workspace with two members. The binary's `src/` stays at the workspace root; the library is a sibling directory `tui_pane/` rather than nested under a `crates/` wrapper. Reasons: (a) zero path churn for the binary (`cargo install --path .`, `git blame`, IDE indexes, CI scripts unchanged); (b) keeps the top level concise — only two crate directories — without an extra one-entry `crates/` directory; (c) the root `Cargo.toml` is both a `[package]` and a `[workspace]`, which is well-supported by cargo and exercises the package-workspace pattern.

```
cargo-port-api-fix/
├── Cargo.toml                      # [package] (binary) + [workspace] (members = ["tui_pane"])
├── src/                            # cargo-port binary crate (unchanged location)
│   └── …
└── tui_pane/                       # library crate
    ├── Cargo.toml
    └── src/
        ├── lib.rs                  # public API re-exports
        ├── keymap/
        │   ├── mod.rs              # Keymap<Ctx>; scope_for/navigation/globals lookups
        │   ├── key_bind.rs         # KeyBind, From<KeyCode>, From<char>, shift/ctrl,
        │   │                       #   display, display_short, parsing
        │   ├── bindings.rs         # Bindings<A> builder + bindings! macro
        │   ├── scope_map.rs        # ScopeMap<A>, display_keys_for,
        │   │                       #   primary-key invariants
        │   ├── action_enum.rs     # Action trait + action_enum! macro
        │   ├── global_action.rs   # GlobalAction enum + Action impl
        │   │                       #   + GlobalAction::defaults() (added Phase 4)
        │   ├── shortcuts.rs        # Shortcuts<Ctx> trait (Phase 7)
        │   ├── navigation.rs       # Navigation<Ctx> trait (Phase 7)
        │   ├── globals.rs          # Globals<Ctx> trait (Phase 7)
        │   ├── builder.rs          # KeymapBuilder<Ctx, State>; register,
        │   │                       #   register_navigation, register_globals,
        │   │                       #   with_settings, vim_mode,
        │   │                       #   builder(quit, restart, dismiss)
        │   ├── vim.rs              # VimMode enum (Phase 3); vim-binding
        │   │                       #   application + vim_mode_conflicts +
        │   │                       #   is_vim_reserved fns added in Phase 10
        │   └── load.rs             # TOML parsing, scope replace semantics,
        │                           #   collision errors, config_path() via dirs
        ├── bar/                    # framework-owned bar renderer
        │   ├── mod.rs              # render() entry; orchestrates regions
        │   │                       #   in BarRegion::ALL order
        │   ├── region.rs           # BarRegion::{ Nav, PaneAction, Global }
        │   ├── slot.rs             # BarSlot<A> + ShortcutState
        │   ├── support.rs          # format_action_keys, push_cancel_row,
        │   │                       #   shared row builders
        │   ├── nav_region.rs       # left: ↑/↓ nav, ←/→ expand, +/- all, Tab pane
        │   ├── pane_action_region.rs # center: per-action rows from focused
        │   │                       #   pane's bar_slots + label/state
        │   └── global_region.rs    # right: GlobalAction + AppGlobals
        ├── settings.rs             # SettingsRegistry<Ctx>;
        │                           #   add_bool / add_enum / add_int
        ├── framework.rs            # Framework<Ctx> aggregator;
        │                           #   mode_queries registry;
        │                           #   editor_target_path,
        │                           #   focused_pane_mode
        └── panes/                  # framework-internal panes
            ├── mod.rs
            ├── keymap_pane.rs      # KeymapPane<Ctx>;
            │                       #   EditState::{Browse, Awaiting, Conflict}
            ├── settings_pane.rs    # SettingsPane<Ctx>;
            │                       #   EditState::{Browse, Editing}
            └── toasts.rs           # Toasts<Ctx>; ToastsAction::Dismiss
```

App-specific code stays in the binary crate. Framework code lives only in `tui_pane/src/`.

**Conceptual module dependencies** (Rust modules within one crate compile as a unit, so this is a readability layering, not a hard ordering):
- `keymap/` — bindings storage + traits + builder. The builder is the keymap's builder; it calls into `framework.rs` and `settings.rs` to file pane queries and settings during registration, but the resulting `Keymap<Ctx>` is the build product.
- `bar/` — reads `Keymap<Ctx>` and pane `Shortcuts<Ctx>` impls; emits `StatusBar`.
- `panes/` — framework panes implementing `Shortcuts<Ctx>`.
- `settings.rs` — `SettingsRegistry<Ctx>`.
- `framework.rs` — aggregates framework panes and the `mode` query registry.
- `lib.rs` — public re-exports.

Orphan-rule note: `Shortcuts<Ctx>` is a foreign trait when used by the binary, so app pane types must be defined in the binary crate (or any crate that owns the type — third-party panes need their own crate). Cargo-port has no third-party panes, so this constraint never fires.

---

## Trait family

Three traits, one per scope flavor.

### Framework generic parameter `Ctx: AppContext`

`tui_pane` is a separate workspace crate; it cannot name the binary's `App` type directly. The framework is generic over an app-context type the binary supplies, bounded by the `AppContext` trait. The binary supplies `App` and writes `impl AppContext for App`.

`AppContext` (in `tui_pane`) carries the cross-cutting plumbing the framework needs:

```rust
pub trait AppContext: 'static {
    type AppPaneId: Copy + Eq + Hash + Debug + 'static;
    fn framework(&self) -> &Framework<Self>;
    fn framework_mut(&mut self) -> &mut Framework<Self>;
    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>);
}
```

Focus reads happen on `Framework<Ctx>` (`framework.focused()`); focus writes go through `AppContext::set_focus`. Framework code reads `framework.focused` directly without calling back through `Ctx`. The binary's `Focus` subsystem (overlay-return memory, visited set, `pane_state`) is the single writer of `framework.set_focused` — every framework-originated transition routes through `ctx.set_focus`, which the binary impls by calling into `Focus`.

The trait does **not** require `Ctx` to expose pane state — every pane's own state is reached via the per-pane dispatcher's free fn navigating through `Ctx` (`&mut ctx.panes.package`, etc.).

For the rest of this doc, signatures use `Ctx` (or `Ctx: AppContext`) when referring to the app context. See `paneid-ctx.md` §2 for the full trait.

### Pane id design

Two enums + a wrapping type:

- `tui_pane::FrameworkPaneId { Keymap, Settings, Toasts }` — the framework's three internal panes. Always written `FrameworkPaneId` (no inside-the-crate `PaneId` short form) so the type name is one-to-one across binary and library.
- `cargo_port::AppPaneId { Package, Git, ProjectList, … }` — cargo-port's 10 panes. Hand-written enum in `src/tui/panes/spec.rs` (today's enum, minus the framework variants).
- `tui_pane::FocusedPane<AppPaneId> { App(AppPaneId), Framework(FrameworkPaneId) }` — generic wrapper used in framework trait signatures. The binary uses this directly for focus tracking.

Linking the runtime tag to the compile-time pane type: every `Pane<App>` impl declares `const APP_PANE_ID: AppPaneId`. Calling `register::<PackagePane>()` records that value alongside the pane's dispatcher — registration populates the runtime mapping. The `AppPaneId` enum is the runtime side of the same registration.

Cargo-port's existing `tui::panes::PaneId` enum becomes a type alias `pub type PaneId = tui_pane::FocusedPane<AppPaneId>;` so existing call sites that name `PaneId` keep compiling; only the framework variants move out of the enum body.

See `paneid-ctx.md` §1 for full type definitions and call-site rewrites.

### `Pane` and `Shortcuts` — per-pane traits

`Pane<Ctx>` carries the pane identity and per-frame mode. `Shortcuts<Ctx>: Pane<Ctx>` adds the shortcut-config surface — bindings, bar layout, dispatcher. Splitting them lets a pane (e.g. a pure text-input overlay) impl `Pane<Ctx>` without `Shortcuts<Ctx>`, and lets future per-pane traits (`MouseInput<Ctx>: Pane<Ctx>`, etc.) be added without bloating `Shortcuts`.

```rust
pub trait Pane<Ctx: AppContext>: 'static {
    /// Stable per-pane identity; keys the framework's per-pane registries.
    const APP_PANE_ID: Ctx::AppPaneId;

    /// Per-frame mode. Framework reads on every key event and uses the
    /// result to gate region suppression, structural Esc, and key
    /// dispatch. `TextInput(handler)` carries the handler inline so
    /// "TextInput without a handler" is unrepresentable. Default
    /// returns `Navigable`.
    fn mode() -> fn(&Ctx) -> Mode<Ctx> {
        |_ctx| Mode::Navigable
    }
}

pub enum Mode<Ctx> {
    /// No scrolling, no typed input (Output, Toasts, KeymapPane Conflict).
    /// `Nav` region suppressed; `Global` emitted.
    Static,

    /// List/cursor — `NavigationAction` drives it; framework emits
    /// the `Nav` region. The default mode for app panes.
    Navigable,

    /// Pane consumes typed characters (Finder, SettingsPane Editing,
    /// KeymapPane Awaiting). `Nav` and `Global` regions both
    /// suppressed; structural Esc pre-handler also suppressed. The
    /// handler is the sole authority for keys while the pane is in
    /// this mode — there is no fall-through to global dispatch. To
    /// exit, the handler mutates `ctx` so `mode()` next frame returns
    /// `Navigable`/`Static`. To honor any global key (Ctrl+Q, etc.),
    /// the handler implements it itself.
    TextInput(fn(KeyBind, &mut Ctx)),
}

pub trait Shortcuts<Ctx: AppContext>: Pane<Ctx> {
    type Actions: Action + 'static;
    const SCOPE_NAME: &'static str;

    fn defaults() -> Bindings<Self::Actions>;

    /// Per-frame visibility for `action`. `Hidden` removes the slot
    /// from the bar entirely; `Visible` (default) renders it. Use this
    /// instead of returning `None` from a label override — the bar
    /// label itself is always `Action::bar_label()` and never varies
    /// at runtime.
    fn visibility(&self, _action: Self::Actions, _ctx: &Ctx) -> Visibility {
        Visibility::Visible
    }

    /// Per-frame enabled / disabled status for `action`. Default
    /// `Enabled`. Override when the action is visible but inert (e.g.
    /// `PackageAction::Clean` grayed out when no target dir exists).
    fn state(&self, _action: Self::Actions, _ctx: &Ctx) -> ShortcutState {
        ShortcutState::Enabled
    }

    /// Bar slot layout. Owned `Vec`; cheap (N ≤ 10) and ratatui's
    /// per-frame work dwarfs the allocation. Each slot carries the
    /// `BarRegion` it lands in; most panes return
    /// `(BarRegion::PaneAction, Single(action))` for every action, but
    /// ProjectList additionally returns `(BarRegion::Nav, Paired(…))`
    /// for its expand/collapse pairs. Default impl returns one
    /// `(PaneAction, Single(action))` per `Self::Actions::ALL` in
    /// declaration order; override to introduce paired slots, route
    /// into `Nav`, or to omit data-dependent slots.
    fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        Self::Actions::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Optional vim-extras: pane actions that should also bind to a
    /// keybind when `VimMode::Enabled`. Default empty. Used by
    /// `ProjectListAction::ExpandRow` (binds `'l'`) /
    /// `CollapseRow` (binds `'h'`). `KeyBind` (not `char`) so future
    /// extras can include modifier keys.
    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &[] }

    /// Returns a free function the framework calls to dispatch an
    /// action. The function takes `&mut Ctx` so the framework holds
    /// the only `&mut` borrow during dispatch (split-borrow: framework
    /// cannot hold `&mut self` from inside `&mut Ctx` while also
    /// passing `&mut Ctx`). Each pane registers a free function:
    ///
    /// ```rust
    /// fn dispatch_package(action: PackageAction, app: &mut App) { … }
    /// impl Shortcuts<App> for PackagePane {
    ///     fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch_package }
    /// }
    /// ```
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Visibility {
    Visible,
    Hidden,
}
```

A pane writes both `impl Pane<App> for PackagePane` and `impl Shortcuts<App> for PackagePane`.

- **`Pane::APP_PANE_ID`** — stable per-pane identity used by the framework's per-pane registries.
- **`Pane::mode`** — returns a `fn(&Ctx) -> Mode<Ctx>`. Default `Navigable`. Panes whose mode varies override (Finder returns `TextInput(finder_keys)` while open, `Navigable` otherwise).
- **`Shortcuts::SCOPE_NAME`** — TOML table name; survives type renames; one-line cost. Required.
- **`Shortcuts::defaults`** — pane's default bindings. No framework default (every pane has its own keys).
- **`Shortcuts::visibility(action, ctx) -> Visibility`** — `Visible` shows the slot, `Hidden` removes it. Default `Visible`; override for state-dependent visibility (CiRuns Activate at EOL hidden when no rows). The bar **label** itself is always `Action::bar_label()` declared in `action_enum!` — there is no per-frame label override.
- **`Shortcuts::state(action, ctx) -> ShortcutState`** — `Enabled` (lit) or `Disabled` (grayed). Default `Enabled`; override when the action is visible but inert.
- **`Shortcuts::bar_slots(ctx)`** — declares the slot layout per-frame. Most panes accept the default (one slot per action, declaration order). Panes with paired slots override (ProjectList: `↑/↓ nav`, `←/→ expand`, `+/- all`). Data-dependent omission lives here too.
- **`Shortcuts::vim_extras`** — pane-action vim bindings (separate from `Navigation`'s arrow → vim mapping).
- **`Shortcuts::dispatcher`** — returns a free function pointer. Framework calls `dispatcher()(action, ctx)`.

### `BarSlot` enum

```rust
pub enum BarSlot<A> {
    Single(A),                  // one action, full key list shown via display_short joined by ','
    Paired(A, A, &'static str), // two actions glued with `/`, one shared label, primary keys only
}
```

Framework rendering:
- `Single(action)` → renders all keys bound to `action` (joined by `,` after `display_short`) `<space>` `action.bar_label()`. Slot is hidden when `pane.visibility(action, ctx) == Hidden`; grayed when `pane.state(action, ctx) == Disabled`.
- `Paired(left, right, label)` → renders `display_short(left.primary) "/" display_short(right.primary) <space> label`. **Primary keys only — alternative bindings for paired actions never appear in paired slots.** Used for `↑/↓ nav`, `←/→ expand`, `+/- all`, `←/→ toggle`.

`KeyBind::display_short` for any key intended to render in a paired slot must not produce a string containing `,` or `/`. The framework `debug_assert!`s this in `Paired` rendering and a Phase 2 unit test walks every `KeyCode` variant via `display_short` to confirm.

### `Navigation` — declarative, single instance per app

```rust
pub trait Navigation<Ctx: AppContext> {
    type Actions: Action + 'static;
    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Actions;
    const DOWN:  Self::Actions;
    const LEFT:  Self::Actions;
    const RIGHT: Self::Actions;
    fn defaults() -> Bindings<Self::Actions>;

    /// Free function the framework calls when any navigation action fires.
    /// `focused` lets the app dispatch to whichever scrollable surface
    /// owns the focused pane. One match arm per action, mirroring the
    /// `Shortcuts::dispatcher` and `Globals::dispatcher` pattern.
    fn dispatcher() -> fn(Self::Actions, FocusedPane<Ctx::AppPaneId>, &mut Ctx);
}
```

Pane scopes carry per-instance state and dispatch logic; their bar contribution depends on that state. `Navigation` has neither (no per-instance state) but it does need dispatch, since the focused pane needs to scroll on `Up`. The framework reads the four required consts to render the nav row and to apply vim-mode bindings, and calls `dispatcher()(action, focused, ctx)` to route.

### `Globals` — declarative, app extension scope

```rust
pub trait Globals<Ctx: AppContext> {
    type Actions: Action + 'static;
    const SCOPE_NAME: &'static str = "global";
    fn render_order() -> &'static [Self::Actions];
    fn defaults() -> Bindings<Self::Actions>;
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
}
```

The app's *additional* globals scope (Find, OpenEditor, Rescan, etc.). The framework's pane-management/lifecycle globals are owned separately by `GlobalAction` (below); the app does not redefine them.

---

## `Keymap<Ctx>` runtime container

> Formal API in `core-api.md` §6 (`Keymap<Ctx>`) and §7 (`KeymapBuilder<Ctx>`, including the type-state choice, `KeymapError` variants, required vs optional methods). Per Phase 9 review: `KeymapError` is the one error type spanning loader and builder validation; `BuilderError` was dropped from the design.

Struct name: `Keymap<Ctx>`. Built once at startup, read every frame. Framework-owned, internally TypeId-keyed.

```rust
pub struct Keymap<Ctx> { /* private TypeId-keyed map */ _ctx: PhantomData<fn(&mut Ctx)> }

impl<Ctx> Keymap<Ctx> {
    pub fn builder() -> KeymapBuilder<Ctx> { … }

    /// Returns the binding map for a pane scope.
    pub fn scope_for<P: Shortcuts<Ctx>>(&self) -> &ScopeMap<P::Action> { … }

    /// Returns the navigation scope's binding map.
    pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Action> { … }

    /// Returns the app-globals scope's binding map.
    pub fn globals<G: Globals<Ctx>>(&self) -> &ScopeMap<G::Action> { … }

    /// Framework's globals scope. App may rebind keys via TOML
    /// `[global]`; loader merges with `AppGlobals` in the same scope
    /// (see TOML grammar). Framework-internal access is `pub(crate)`.
    pub(crate) fn globals(&self) -> &ScopeMap<GlobalAction> { … }
}
```

The binary types `Keymap<App>` everywhere it consumes the framework. Framework-internal code uses `Keymap<Ctx>`. Adding a new scope = registering a new pane; no central struct edit.

All `Shortcuts<Ctx>` / `Navigation<Ctx>` / `Globals<Ctx>` references in this doc are explicit; any bare `Shortcuts` (without `<Ctx>`) is a doc-level shorthand for "the trait" rather than a fresh trait declaration.

### Registration

```rust
let keymap = Keymap::<App>::builder(quit, restart, dismiss)
    .vim_mode(VimMode::Enabled)
    .register::<PackagePane>()              // each pane's Shortcuts impl
    .register::<GitPane>()
    .register::<ProjectListPane>()
    .register::<CiRunsPane>()
    .register::<LintsPane>()
    .register::<TargetsPane>()
    .register::<OutputPane>()
    .register::<LangPane>()
    .register::<CpuPane>()
    .register::<FinderPane>()
    .with_settings(|registry| {
        registry.add_bool(
            "invert_scroll", "Invert scroll",
            |app: &App| app.config.invert_scroll,
            |app: &mut App, value| app.config.invert_scroll = value,
        );
        registry.add_enum(
            "navigation_keys", "Navigation keys",
            &["arrows_only", "arrows_and_vim"],
            |app| app.config.navigation_keys.toml_key(),
            |app, value| app.config.navigation_keys = NavigationKeys::from_toml_key(value),
        );
        registry.add_int(
            "ci_run_count", "CI runs to fetch",
            |app| app.config.ci_run_count as i64,
            |app, value| app.config.ci_run_count = value as u32,
        );
    })
    .register_navigation::<AppNavigation>(AppNavigation)
    .register_globals::<AppGlobals>(AppGlobals)
    .load_toml(Keymap::config_path("cargo-port"))?
    .build();
```

Stored on the app (`app.keymap`); passed as `&Keymap` to framework dispatch / bar code.

---

## `ShortcutState` (returned by `Shortcuts::state`)

```rust
pub enum ShortcutState { Enabled, Disabled }
```

The bar's two orthogonal axes are split across two `Shortcuts` methods:

- `label(action, ctx) -> Option<&'static str>` — the verb shown next to the key. `None` hides the slot. Default returns `Some(action.bar_label())`; pane overrides only when the label depends on pane state.
- `state(action, ctx) -> ShortcutState` — `Enabled` (lit) or `Disabled` (grayed). Default `Enabled`; pane overrides when the action is visible but inert.

The framework adds the bound key (looked up via `display_keys_for(action)`) when rendering. The pane never builds a key string. The label and state are independent: a slot can be hidden, lit, or grayed, but the label string never carries the enabled/disabled bit.

---

## Key types

- `crossterm::event::KeyCode` used directly. No alias.
- `crossterm::event::KeyModifiers` used directly. No alias.

`KeyBind` is the framework's bundle:

```rust
pub struct KeyBind { pub code: KeyCode, pub mods: KeyModifiers }

impl From<KeyCode>  for KeyBind { … }  // KeyCode::Enter → KeyBind { Enter, NONE }
impl From<char>     for KeyBind { … }  // 'c'           → KeyBind { Char('c'), NONE }

impl KeyBind {
    pub fn shift(into: impl Into<Self>) -> Self { … } // OR-composes with `ctrl`
    pub fn ctrl(into:  impl Into<Self>) -> Self { … }
    pub fn display_short(&self) -> String { … } // arrows → glyphs (↑/↓/←/→), else display name
    pub fn display(&self) -> String { … }       // canonical Ctrl+Alt+Shift+key
}

// Kind-tagged event-loop wrapper. Keymap dispatch only handles Press;
// Release/Repeat flow through for handlers that opt in.
pub enum KeyInput { Press(KeyBind), Release(KeyBind), Repeat(KeyBind) }
impl KeyInput {
    pub const fn from_event(event: KeyEvent) -> Self { … } // crossterm bridge
    pub const fn bind(&self)  -> &KeyBind          { … }
    pub const fn press(&self) -> Option<&KeyBind>  { … } // Some only on Press
}
```

International-character support: crossterm's `KeyCode::Char(char)` already covers Unicode codepoints, so `char`-based bindings compose correctly without the framework defining its own enum.

---

## `Bindings<A>` builder

> Formal API in `core-api.md` §2; macro grammar + expansion in `macros.md` §1.

```rust
pub struct Bindings<A> { … }

impl<A> Bindings<A> {
    pub fn new() -> Self { … }
    pub fn bind(&mut self, key: impl Into<KeyBind>, action: A) -> &mut Self;
    pub fn bind_many(&mut self, keys: impl IntoIterator<Item = KeyBind>, action: A)
        -> &mut Self;
}
```

### `bindings!` macro

Single-key and multi-key with the same arrow:

```rust
bindings! {
    KeyCode::Enter      => PackageAction::Activate,
    'c'                 => PackageAction::Clean,
    [KeyCode::Up, 'k']  => NavigationAction::Up,
    ['=', '+']          => ProjectListAction::ExpandAll,
    '-'                 => ProjectListAction::CollapseAll,
    KeyBind::shift('g') => SettingsAction::ToggleNext,
    KeyBind::ctrl('k')  => GlobalAction::OpenKeymap,
}
```

Macro expands to `Bindings<A>` populated via `bind` / `bind_many`. List form binds multiple keys to one action; both keys dispatch independently. The first key in a list is the **primary** (what the bar renders when only one key is shown).

The TOML parser accepts the same single-or-array form: `key = "Enter"` or `key = ["Enter", "Return"]`. In-array duplicates are rejected.

### Dropped: `parse_keybind` `+`/`=` collapse

Today's `parse_keybind` at `keymap.rs:140-142` maps both TOML strings `"="` and `"+"` to `KeyCode::Char('+')` (a parsing-time quirk to avoid conflict with TOML's `+` modifier separator). With multi-bind, drop it: `"="` → `KeyCode::Char('=')`, `"+"` → `KeyCode::Char('+')`. The user binds both to the same action explicitly when they want either physical key to fire.

---

## `ScopeMap<A>` — multi-bind support

> Formal API in `core-api.md` §3.

`by_key` stays 1-to-1 within a scope (dispatch). `by_action` becomes `HashMap<A, Vec<KeyBind>>`:

```rust
pub struct ScopeMap<A: Copy + Eq + Hash> {
    pub(crate) by_key:    HashMap<KeyBind, A>,
    pub(crate) by_action: HashMap<A, Vec<KeyBind>>,
}

impl<A: Copy + Eq + Hash> ScopeMap<A> {
    pub fn insert(&mut self, key: KeyBind, action: A) {
        debug_assert!(
            self.by_key.get(&key).is_none_or(|&existing| existing == action),
            "ScopeMap::insert: key {key:?} already maps to a different action",
        );
        self.by_key.insert(key.clone(), action);
        self.by_action.entry(action).or_default().push(key);
    }

    pub fn action_for(&self, key: &KeyBind) -> Option<A> { … } // dispatch
    pub fn key_for(&self, action: A) -> Option<&KeyBind> { … } // primary
    pub fn display_key_for(&self, action: A) -> String { … }   // primary, full name
    pub fn display_keys_for(&self, action: A) -> &[KeyBind] { … } // all, insertion order
}
```

### Display-string and primary-key invariants

`KeyBind::display_short` maps arrow keys to glyphs (`Up → "↑"`, `Down → "↓"`, etc.) and otherwise delegates to `display`. The **bar uses `display_short`**; the keymap-overlay UI keeps `display` (full names suit the help screen better).

`defaults()` insertion-order rule: for any action that may bind to both an arrow key and a vim key, **insert the arrow key first** so it's primary. Tests lock this: `key_for(NavigationAction::Up) == KeyBind::from(KeyCode::Up)` even when vim mode is on.

Invariant: every key in `by_key` appears exactly once across all `by_action` vecs. Test: `by_key.len() == by_action.values().map(Vec::len).sum::<usize>()`.

---

## Framework panes vs app panes

**Framework panes** (in `tui_pane`):

- `KeymapPane` — viewer/editor for the registered keymap. Sub-states (Browse / Awaiting / Conflict) are mode-flag internals the app never sees.
- `SettingsPane` — generic settings UI (browse list + edit individual values). Sub-states (Browse / Editing) are mode-flag internals.
- `Toasts` — notification stack.

The app does not write `Shortcuts` impls for these — they ship with their own internal impls in `tui_pane`.

**App panes** (cargo-port specific): `PackagePane`, `GitPane`, `ProjectListPane`, `CiRunsPane`, `LintsPane`, `TargetsPane`, `OutputPane`, `LangPane`, `CpuPane`, `FinderPane`. App writes `Shortcuts` impls for these.

(`FinderPane` is technically a generic search overlay over project content — it could become framework in a follow-up, but it currently knows enough about cargo-port projects to stay app-side for now.)

### Sub-state panes — internal mode flag, not separate panes

When a single pane has multiple modes (Browse vs Editing for Settings, Browse vs Awaiting vs Conflict for Keymap), use an internal `EditState` flag and route via `visibility()` / `state()` / `dispatch()`. Do **not** create a separate `*Pane` type per mode.

Mode-neutral action names (`Activate`, `Cancel`, `Left`, `Right`) describe the user's intent; the pane decides what each intent does given its current `EditState`. Action labels are static (`Action::bar_label()`); per-mode label variation is expressed by introducing distinct action variants (e.g. `BeginEdit`/`ConfirmEdit`) rather than overriding labels per state. `visibility` and `state` filter which actions appear/are-live in each `EditState`:

```rust
impl Shortcuts<App> for SomePane {
    fn visibility(&self, action: SomeAction, _ctx: &App) -> Visibility {
        match (self.edit_state, action) {
            (EditState::Browse, SomeAction::ConfirmEdit) => Visibility::Hidden,
            (EditState::Edit,   SomeAction::BeginEdit)   => Visibility::Hidden,
            _ => Visibility::Visible,
        }
    }

    fn dispatcher() -> fn(SomeAction, &mut App) { dispatch_some }
}

fn dispatch_some(action: SomeAction, ctx: &mut App) {
    let pane = &mut ctx.panes.some;
    match (&mut pane.mode, action) {
        (Mode::Browse, SomeAction::Activate) => pane.enter_edit_mode(),
        (Mode::Edit,   SomeAction::Activate) => pane.commit_edit(),
        // …
    }
}
```

For framework panes (Keymap, Settings) this pattern is internal to `tui_pane`. For app panes, this pattern is rare in practice — most app panes have one mode.

Note: the `dispatch` example uses free-fn form (`fn dispatch_some(action, ctx)`) to match the `Shortcuts::dispatcher` rule. Pane state is reached via `&mut ctx.panes.some` — the dispatcher navigates from the `Ctx` root rather than holding `&mut self` directly.

### `InputContext` disappears

The existing `InputContext` enum collapses entirely. Focus = pane. The framework's pane registry tracks which registered panes are overlays (set via builder metadata at registration time); that's the only "context" still needed (it controls global-strip visibility during overlays).

---

### Closure-vs-free-fn rule

`Shortcuts::dispatcher` / `Navigation::dispatcher` / `Globals::dispatcher` and `with_quit` / `with_restart` / `with_dismiss` use **free function pointers**, not closures. Reason: the framework holds `&mut Ctx` while calling the dispatcher, so a closure capturing any `Ctx`-derived reference would create a re-entrant borrow.

`with_settings(|registry| …)` and `add_bool` / `add_enum` / `add_int` get/set closures use **closures**. Reason: settings get/set closures *are* the borrow holder — the framework calls them during dispatch and gives them `&Ctx` / `&mut Ctx` directly. There's no re-entrancy hazard. Use closures here for ergonomics.

Rule: free fn whenever the framework holds `&mut Ctx` while invoking your callback; closure whenever your callback *is* the `&mut Ctx` borrow.

## Settings registry — framework UI, app data

> Formal API in `core-api.md` §9 (closure signatures, persistence semantics).

Each setting carries: TOML key, display label, value-getter closure, value-setter closure. Three value flavors covered: `bool`, `enum` (closed string set), `int` (with optional min/max).

The app provides only data + closures. It writes no `Shortcuts` impl, no mode state machine, no overlay rendering.

---

## `GlobalAction` — framework base, app extension

> Formal enum + dispatch hooks in `core-api.md` §10. TOML grammar for the merged `[global]` table in `toml-keys.md` §1–2.

Framework owns pane-management, lifecycle, and the framework overlays:

```rust
// tui_pane
pub enum GlobalAction {
    Quit,
    Restart,
    NextPane,
    PrevPane,
    OpenKeymap,    // focus framework's KeymapPane overlay
    OpenSettings,  // focus framework's SettingsPane overlay
    Dismiss,       // close current overlay or dismiss top dismissable
}
```

Framework owns defaults (`q` → Quit, `R` → Restart, `Tab` → NextPane, `Shift+Tab` → PrevPane, `Ctrl+K` → OpenKeymap, `s` → OpenSettings, `x` → Dismiss), the bar entries, **and dispatch for all seven variants** (post-Phase-3 review decision).

### Framework-owned dispatch + optional binary hooks

Per the Phase 3 review, the framework owns dispatch for every `GlobalAction` variant. The binary opts in to *notification* via three optional builder hooks; all default to no-op.

| Variant            | Framework behavior                                                                 | Binary opt-in                              |
|--------------------|------------------------------------------------------------------------------------|--------------------------------------------|
| `Quit`             | Sets `Framework<Ctx>::quit_requested = true`. Binary's main loop polls and exits.  | `.on_quit(\|app\| { /* save state */ })`   |
| `Restart`          | Sets `Framework<Ctx>::restart_requested = true`. Binary's main loop polls.         | `.on_restart(\|app\| { /* save state */ })`|
| `Dismiss`          | Runs framework dismiss chain: top toast, then focused framework overlay. If nothing dismissed, calls binary's `dismiss_fallback`. | `.dismiss_fallback(\|app\| -> bool { app.try_dismiss_focused_app_thing() })` |
| `NextPane`         | Pure pane-focus — framework knows the registered pane set.                         | (none — binary doesn't see this)           |
| `PrevPane`         | Pure pane-focus.                                                                   | (none)                                     |
| `OpenKeymap`       | Focuses framework's `KeymapPane` overlay.                                          | (none)                                     |
| `OpenSettings`     | Focuses framework's `SettingsPane` overlay.                                        | (none)                                     |

```rust
Keymap::<App>::builder()
    .on_quit(|app| { app.persist_state() })           // optional
    .on_restart(|app| { app.persist_state() })        // optional
    .dismiss_fallback(|app| -> bool {                 // optional
        app.try_dismiss_focused_app_thing()
    })
    .vim_mode(VimMode::Enabled)
    .register::<PackagePane>()
    // …
```

The dismiss chain rationale: the mouse-click hit-test for the X button on framework overlays already lives in the framework. Splitting Esc-key dismiss between framework (overlays) and binary (everything else) duplicates that logic. One owner — framework — for both Esc and mouse, with a one-fn fallback for app-level dismissables.

### App globals

App declares its own additional globals:

```rust
// cargo-port
pub enum AppGlobalAction { Rescan, OpenEditor, OpenTerminal, Find }
impl Globals<App> for AppGlobalAction { … }
```

Both `GlobalAction` and `AppGlobalAction` share a single TOML table named `[global]`. The loader matches each TOML key against both enums; whichever variant accepts the key is the action that gets bound. From the user's perspective there's one globals namespace — they write `[global] quit = "q"` (framework variant) and `[global] find = "/"` (app variant) into the same table.

If the same TOML key resolves in **both** enums (e.g. binary's `AppGlobalAction` defines a variant whose `toml_key()` collides with one of `GlobalAction`'s seven), the loader emits `KeymapError::CrossEnumCollision { key, framework_action, app_action }` at load time. This is a definition-time error — the app dev must rename the colliding `toml_key` string in `AppGlobalAction` before the binary can run. The framework's seven keys are stable, so the rename is always app-side.

Bar's right-hand strip: framework renders `GlobalAction` items first, then `AppGlobals::render_order()`.

### Per-action revert policy for `[global]`

The `[global]` scope uses a more permissive error policy than other scopes (which fully replace on present, defaults on absent). Per-action behavior:

- Each TOML entry in `[global]` is processed independently.
- If the value parses cleanly and doesn't collide with another binding in `[global]`, apply it.
- If the value fails to parse OR collides at the binding level → emit a warning, revert *just that action* to its default, continue processing the rest of the table. (Cross-enum `toml_key` collision is a definition-time error, not a per-binding revert — see above.)
- Framework-owned actions (`Quit`, `Restart`, `Dismiss`) that the user accidentally drops or invalidates are restored to their defaults at the end of the pass — the framework always has working lifecycle keys.

Result: the framework always has working base globals, while the user's customizations to `Find` / `OpenEditor` / etc. survive intact even if one binding broke. Other scopes (per-pane, navigation) keep the simpler "TOML replaces entirely" rule.

---

## Vim mode — framework capability

Framework owns vim-mode handling. The app passes a flag at builder time (`VimMode::Enabled` / `VimMode::Disabled`).

App writes arrow-key defaults in `Navigation::defaults()`. Vim bindings are applied **inside `KeymapBuilder::build()`** in this order:

1. Merge each registered scope's `defaults()` into the per-action binding map. Arrow keys land first.
2. Apply user TOML. **TOML replaces, doesn't merge** — a TOML table for a scope completely overrides that scope's bindings. Tables not present in TOML keep defaults.
3. If `VimMode::Enabled`: append `'k'/'j'/'h'/'l'` to `Navigation::UP/DOWN/LEFT/RIGHT` (skipping any already bound). Walk every registered `Shortcuts` impl's `vim_extras()` and append (`ProjectListAction::ExpandRow → 'l'`, `CollapseRow → 'h'`).

Vim is applied **after** TOML so the user's `[navigation] up = ["PageUp"]` doesn't disable vim — the extras still apply on top. Arrow keys remain primary because they were inserted first in step 1 (when present); user TOML reorders primary if it replaces.

`vim_mode_conflicts` is also framework — walks registered scopes to check for `h/j/k/l` already bound to non-navigation actions.

`is_vim_reserved` reads `Navigation`'s actual bindings (the trait's required consts plus the resolved scope) instead of a hardcoded `VIM_RESERVED` table.

### Binding-capture reservation (KeymapAwaiting)

Today `keymap_ui.rs:236-250` calls `is_navigation_reserved` during binding capture to reject Up/Down/etc. Post-refactor the framework's KeymapPane (in Awaiting mode) reads `keymap.navigation::<AppNavigation>().action_for(&candidate_bind)` (typed singleton getter from Find 13) and rejects when `Some(_)`. Same for vim keys via `is_vim_reserved`. The hardcoded `NAVIGATION_RESERVED` and `VIM_RESERVED` tables disappear.

---

## TOML loading — framework

> Full grammar, error taxonomy, vim-after-TOML worked examples, and `KeyBind::display` / `display_short` mapping tables in `toml-keys.md`.

Framework handles all TOML loading. Each registered scope's `SCOPE_NAME` constant drives table lookup; framework parses every recognized table, replaces that scope's bindings, leaves missing tables at their declared defaults+vim. App provides no TOML hooks.

### TOML errors

- In-array duplicates: `key = ["Enter", "Enter"]` → parse error.
- Cross-action duplicates within a non-globals scope (e.g. `[finder] activate = "Enter"` and `cancel = "Enter"`) → parse error (return `Err`).
- The `[global]` scope follows the per-action revert policy described under `GlobalAction` above — broken individual bindings revert to defaults; the loader returns `Ok` with a list of warnings.

The `ScopeMap::insert` `debug_assert` catches the same conditions for `defaults()` builders; the TOML loader returns them as real errors or warnings.

See `toml-keys.md` for the full grammar, error taxonomy, and worked vim-after-TOML examples.

`keymap_path()` is framework-provided via the `dirs` crate. App supplies its name at builder time:

```rust
let path = Keymap::config_path("cargo-port");
// → {dirs::config_dir()}/cargo-port/keymap.toml
```

`tui_pane` carries no removed-action migration. Binaries that need to handle removed-action TOML keys do so before calling `load_toml`.

---

## `BarRegion` — three-region bar layout

The bar has three left-to-right regions, declared as a public framework enum:

```rust
pub enum BarRegion {
    Nav,        // ↑/↓ nav, ←/→ expand, +/- all, Tab pane — paired-key rows
    PaneAction, // per-action rows from the focused pane
    Global,     // GlobalAction (Quit/Restart/Find/…) + AppGlobals strip
}

impl BarRegion {
    pub const ALL: &[BarRegion] = &[BarRegion::Nav, BarRegion::PaneAction, BarRegion::Global];
}
```

### Region ownership

| Region | Framework provides | Pane provides | Emitted when |
|---|---|---|---|
| `Nav` | nav row from `Navigation::UP/DOWN`, pane-cycle row from `GlobalAction::NextPane` | optional extra paired rows (ProjectList: `←/→ expand`, `+/- all`) | `matches!(mode, Mode::Navigable)` |
| `PaneAction` | nothing | every pane's per-action rows | always |
| `Global` | `GlobalAction` strip + `AppGlobals::render_order()` | nothing | `!matches!(mode, Mode::TextInput(_))` |

A pane indicates *which region* each of its rows lands in. Most pane rows go into `PaneAction`; ProjectList is the rare case where a pane pushes paired rows into `Nav`.

### Framework panes

| Pane | `mode` | `bar_slots` |
|---|---|---|
| `KeymapPane` (Browse) | `Navigable` | `[(PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `KeymapPane` (Awaiting) | `TextInput(keymap_capture_keys)` | `[(PaneAction, Single(Cancel))]` (user is capturing a keystroke) |
| `KeymapPane` (Conflict) | `Static` | `[(PaneAction, Single(Clear)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Browse) | `Navigable` | `[(Nav, Paired(ToggleBack, ToggleNext, "toggle")), (PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Editing) | `TextInput(settings_edit_keys)` | `[(PaneAction, Single(Confirm)), (PaneAction, Single(Cancel))]` |
| `Toasts` | `Static` | `[(PaneAction, Single(Dismiss))]` |

### Trait change

`Shortcuts::bar_slots` returns `(region, row)` pairs:

```rust
fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
    Self::Actions::ALL.iter()
        .copied()
        .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
        .collect()
}
```

The default `(PaneAction, Single(action))` covers the common case. ProjectList overrides to additionally emit:

```rust
(BarRegion::Nav, BarSlot::Paired(CollapseRow, ExpandRow, "expand")),
(BarRegion::Nav, BarSlot::Paired(ExpandAll,   CollapseAll, "all")),
```

### Render orchestration

`bar/mod.rs::render()` calls `pane.bar_slots(ctx)` once, reads `P::mode()(ctx)`, then walks `BarRegion::ALL` and dispatches:

- `BarRegion::Nav` → `nav_region::render(pane, ctx, keymap, &rows)` — emits framework's nav + pane-cycle rows plus any `(Nav, _)` rows from `rows`. Skipped unless `matches!(mode, Mode::Navigable)`.
- `BarRegion::PaneAction` → `pane_action_region::render(pane, ctx, keymap, &rows)` — emits every `(PaneAction, _)` row, calling `pane.visibility(action, ctx)` and `pane.state(action, ctx)` to filter and style each slot (label is `action.bar_label()`).
- `BarRegion::Global` → `global_region::render(keymap, framework)` — emits `GlobalAction` + `AppGlobals::render_order()`. Skipped when `matches!(mode, Mode::TextInput(_))`.

Each region module returns `Vec<Span>`; `mod.rs` joins them left-to-right with framework-owned spacing into a single `StatusBar`.

## Bar architecture — framework-owned

The status bar is a framework feature. App authors write no bar layout code. See the `BarRegion` section above for the three-region model and the `bar/` module structure.

| Concern | Owner |
|---|---|
| Region orchestration | Framework — `bar/mod.rs` walks `BarRegion::ALL` |
| `Nav` region (paired rows from `Navigation` + pane-cycle from `GlobalAction::NextPane`) | Framework — `bar/nav_region.rs`; emitted only when `matches!(P::mode()(ctx), Mode::Navigable)` |
| `PaneAction` region | Framework — `bar/pane_action_region.rs`; emits `(PaneAction, _)` rows from `pane.bar_slots(ctx)`, calling `pane.visibility(action, ctx)` + `pane.state(action, ctx)` to filter and style each slot (label is `Action::bar_label()`) |
| `Global` region (`GlobalAction` + `AppGlobals::render_order()`) | Framework — `bar/global_region.rs`; suppressed when `matches!(P::mode()(ctx), Mode::TextInput(_))` |
| Color / style / spacing | Framework |
| Per-action visibility & enabled state | Pane (via `Shortcuts::visibility` + `Shortcuts::state`; label is `Action::bar_label()`) |
| Snapshot under default bindings | Framework — parity with current cargo-port bar |

What the app controls: which `AppGlobals` actions appear in `render_order()` and their labels (app-defined verbs).

Framework's per-frame call:

```rust
fn render_status_bar<P, N, G, Ctx>(
    pane: &P, ctx: &Ctx, keymap: &Keymap<Ctx>, framework: &Framework<Ctx>,
) -> StatusBar
where
    P: Shortcuts<Ctx>,
    N: Navigation<Ctx>,
    G: Globals<Ctx>;
```

Pane is the focused one. Framework calls `pane.bar_slots(ctx)` once, then walks `BarRegion::ALL`: each region module filters for its own region tag and emits spans. Region rendering consults `P::mode()(ctx)` for suppression (`Nav` skipped unless `Mode::Navigable`; `Global` skipped on `Mode::TextInput(_)`). Result is a single `StatusBar` value the binary draws to the frame.

**Monomorphization boundary:** `render_status_bar` is monomorphized per pane type at the binary's match-on-`focus.current()` site (see "Bar render — concrete dispatch" below). Each instantiation produces a `StatusBar`. The framework never holds a heterogeneous `Vec<BarSlot<dyn Action>>`; pane types are concrete at the call site.

`KeyBind::display_short` for any key intended to render in a paired slot must not produce a string containing `,` or `/`. Today's bindings satisfy this; if a future binding violates it, the pair-separator scheme has to change.

---

## Inventory: every hardcoded key in cargo-port today

Captured for reference; everything in this table flips to keymap-driven by the end of the refactor.

| Today's literal | Source | Currently configurable? |
|---|---|---|
| `enter("X")` | `shortcuts.rs:107`; 5 inline arms + 4 `*_groups` helpers | ❌ |
| `NAV` | `Shortcut::fixed("↑/↓", "nav")` at `:99` | ❌ |
| `ARROWS_EXPAND` | `Shortcut::fixed("←/→", "expand")` at `:100` | ❌ |
| `ARROWS_TOGGLE` | `Shortcut::fixed("←/→", "toggle")` at `:101` | ❌ |
| `TAB_PANE` | `Shortcut::fixed("Tab", "pane")` at `:102` | partial |
| `ESC_CANCEL` | `Shortcut::fixed("Esc", "cancel")` at `:103` | ❌ |
| `ESC_CLOSE` | `Shortcut::fixed("Esc", "close")` at `:104` | ❌ |
| `EXPAND_COLLAPSE_ALL` | `Shortcut::fixed("+/-", "all")` at `:105` | partial |

`finder.rs:567`, `settings.rs:831, 1163`, `keymap_ui.rs:189`: each `handle_*_key` matches `KeyCode::Enter` / `Esc` / `Up` / `Down` / `Left` / `Right` directly.

`keymap.rs:794-799` `NAVIGATION_RESERVED` tabulates the navigation keys but does not bind them — they're matched directly in input handlers.

After the refactor: each row sources from a `*Action` enum, every input handler routes through scope dispatch, and `NAVIGATION_RESERVED` is replaced by `NavigationAction`'s scope.

---

## Cargo-port action enums

Each app pane owns its own `*Action` enum, declared next to the pane. The `action_enum!` macro (re-exported from `tui_pane`) enforces the TOML-key + bar-label + description + `ALL` slice contract. Each variant carries a tuple of three string literals: TOML key, default bar label, keymap-UI description.

```rust
// tui_pane re-export
pub use tui_pane::action_enum;

// src/tui/panes/package.rs
action_enum! {
    pub enum PackageAction {
        Activate => ("activate", "activate", "Open / activate selected field");
        Clean    => ("clean",    "clean",    "Clean target dir");
        //          ↑ TOML key   ↑ bar       ↑ keymap-UI description
    }
}

// existing variants preserved: ProjectListAction, GitAction, TargetsAction,
// CiRunsAction, LintsAction. Each gains its own pane-local file.
```

New action enums for surfaces today driven by hardcoded key matches:

```rust
// app's NavigationAction (one per app — implements Navigation trait)
action_enum! {
    pub enum NavigationAction {
        Up       => ("up",        "up",     "Move cursor up");
        Down     => ("down",      "down",   "Move cursor down");
        Left     => ("left",      "left",   "Move cursor left / collapse");
        Right    => ("right",     "right",  "Move cursor right / expand");
        PageUp   => ("page_up",   "pgup",   "Page up");
        PageDown => ("page_down", "pgdn",   "Page down");
        Home     => ("home",      "home",   "Jump to top");
        End      => ("end",       "end",    "Jump to bottom");
    }
}

action_enum! {
    pub enum FinderAction {
        Activate  => ("activate",   "go",     "Go to selected match");
        Cancel    => ("cancel",     "close",  "Close finder");
        PrevMatch => ("prev_match", "prev",   "Previous match");
        NextMatch => ("next_match", "next",   "Next match");
        Home      => ("home",       "first",  "Jump to first match");
        End       => ("end",        "last",   "Jump to last match");
    }
}

action_enum! {
    pub enum OutputAction {
        Cancel => ("cancel", "close", "Close output");
    }
}

action_enum! {
    pub enum AppGlobalAction {
        Rescan       => ("rescan",        "rescan",   "Rescan projects");
        OpenEditor   => ("open_editor",   "edit",     "Open editor for selected project");
        OpenTerminal => ("open_terminal", "term",     "Open terminal");
        Find         => ("find",          "find",     "Open finder");
    }
}
```

`ProjectListAction` (existing variants: `ExpandAll`, `CollapseAll`, `Clean`) gains:

```rust
ExpandRow   => ("expand_row",   "expand",   "Expand current node");   // today: KeyCode::Right / 'l'
CollapseRow => ("collapse_row", "collapse", "Collapse current node"); // today: KeyCode::Left  / 'h'
```

(Distinct from `ExpandAll`/`CollapseAll` which apply to the whole tree.)

**Settings, Keymap, Toasts action enums** — these now live inside `tui_pane` (framework-owned panes). The cargo-port app does not declare or implement them; their default keys ship with `tui_pane`.

The `SettingsPane`'s `Cancel` action defaults to `[Esc, 's']` (mirroring today's `'s'` close-on-toggle). Keymap and Toasts default Cancels are `Esc` only.

---

## Default bindings — cargo-port

Multi-bind support lets `NavigationAction::Up` bind to both `Up` and `'k'` (vim mode), `Cancel` actions bind to multiple keys, etc.

| Scope | Action | Default key(s) |
|---|---|---|
| `NavigationAction` | `Up` | `Up` (+ `'k'` when `VimMode::Enabled`) |
| `NavigationAction` | `Down` | `Down` (+ `'j'` when `VimMode::Enabled`) |
| `NavigationAction` | `Left` | `Left` (+ `'h'` when `VimMode::Enabled`) |
| `NavigationAction` | `Right` | `Right` (+ `'l'` when `VimMode::Enabled`) |
| `NavigationAction` | `PageUp` | `PageUp` |
| `NavigationAction` | `PageDown` | `PageDown` |
| `NavigationAction` | `Home` | `Home` |
| `NavigationAction` | `End` | `End` |
| `ProjectListAction` | `ExpandRow` | `Right` (+ `'l'` when vim) — *shared with `NavigationAction::Right`; pane scope wins* |
| `ProjectListAction` | `CollapseRow` | `Left` (+ `'h'` when vim) — same |
| `ProjectListAction` | `ExpandAll` | `+`, `=` |
| `ProjectListAction` | `CollapseAll` | `-` |
| `FinderAction` | `Activate` | `Enter` |
| `FinderAction` | `Cancel` | `Esc` |
| `FinderAction` | `PrevMatch` | `Up` |
| `FinderAction` | `NextMatch` | `Down` |
| `FinderAction` | `Home` | `Home` |
| `FinderAction` | `End` | `End` |
| `OutputAction` | `Cancel` | `Esc` |
| `AppGlobalAction` | `Rescan` | `'r'` |
| `AppGlobalAction` | `OpenEditor` | `'e'` |
| `AppGlobalAction` | `OpenTerminal` | `'t'` |
| `AppGlobalAction` | `Find` | `'/'` |

Existing pane scopes' defaults are unchanged. Settings / Keymap / Toasts defaults ship with `tui_pane`.

---

## Scope precedence

`ScopeMap` keeps the 1-to-1 `by_key: HashMap<KeyBind, A>` invariant **within a scope**. The same key in different scopes is fine.

Resolution order at the input router (preserving today's behavior):

1. **Structural pre-handler** — `GlobalAction::Dismiss` when `app.has_dismissable_output()` is true *and* focus is not a text-input pane (today this is the Esc-clears-`example_output` path at `input.rs:112-119`). Gated on `!matches!(framework.focused_pane_mode(ctx), Mode::TextInput(_))` so typed keys can't trigger structural dismiss while the user is typing into Finder.
2. **Overlay-scope** (if focus is an overlay pane: KeymapPane / SettingsPane / FinderPane) — full handler. Toasts is *not* an overlay (`PaneId::is_overlay` excludes it today; same after the refactor).
3. **`GlobalAction`** — Quit, Restart, NextPane/PrevPane, OpenKeymap/OpenSettings, Dismiss.
4. **`AppGlobalAction`** — Find, OpenEditor, OpenTerminal, Rescan.
5. **Focused-pane scope** (`Shortcuts::Action`).
6. **`NavigationAction`** — for list panes, after the pane scope.

When an overlay key matches both the overlay scope and a global, the overlay wins (overlay scope is consulted first). E.g. binding `FinderAction::Activate` to `Tab` while `GlobalAction::NextPane` is also `Tab` makes `Tab` activate the finder match when finder is open, and cycle panes when finder is closed.

`NavigationAction::Right` and `ProjectListAction::ExpandRow` both default to `Right`/`'l'`: in ProjectList focus the pane scope is consulted before `NavigationAction`, so `Right` fires `ExpandRow`. In every other pane, `Right` falls through to `NavigationAction::Right` (a horizontal no-op for list panes — matching existing behavior).

### Vim-mode `'k'`-in-finder regression: prevented

Today `normalize_nav` (`input.rs:162-165`) early-returns when finder or settings-editing is open, so vim hjkl never converts to arrow keys in those contexts. Post-refactor, `NavigationAction::Up` may default to `[Up, 'k']` (vim mode on). If the finder handler consulted `NavigationAction` unconditionally, typing `'k'` into the search box would fire Up — a regression.

The fix: text-input panes (Finder query, Settings edit-numeric) define their own navigation actions inside their own scope rather than reaching into `NavigationAction`. `FinderAction::PrevMatch / NextMatch / Home / End` cover finder's match-list movement; the finder handler consults only its own scope (and the text-input fall-through for `Char(c)`). `NavigationAction` is never queried, so vim bindings cannot leak into the search box.

### Toasts dismiss precedence

`GlobalAction::Dismiss` defaults to `'x'` and runs in the global pass. `Toasts::ToastsAction::Dismiss` defaults to `Esc`. Result:

- `'x'` on Toasts → `GlobalAction::Dismiss` (global runs first) → existing dismiss path.
- `Esc` on Toasts → no global match, reaches Toasts pane scope → `ToastsAction::Dismiss` fires → same path.

No double-binding conflict, no `'x'` shadow. The bar's per-Toasts row sources `display_key_for(ToastsAction::Dismiss)` which renders `Esc`. The `'x'` global binding is invisible in the Toasts bar but still works.

---

## Bar render — concrete dispatch

`render.rs:531-558` today calls `app.input_context()`-driven `for_status_bar`. Post-deletion, the framework call dispatches off `app.focus.current()` (split between `PaneId::App(_)` and `PaneId::Framework(_)` per the wrapper enum). See `paneid-ctx.md` §1 for the canonical dispatch site — the framework's three panes are routed through a single `bar::render_framework(id, ...)` arm rather than enumerated inline.

The `Settings` / `Keymap` / `Toasts` panes use their internal mode flags (Browse/Editing, Browse/Awaiting/Conflict, etc.) to vary `bar_slots` and `shortcut` output. The current `InputContext::SettingsEditing` / `KeymapAwaiting` / `KeymapConflict` arms collapse into pane-internal mode dispatch.

`overlay_editor_target_path` (`input.rs:413`) becomes `app.framework.editor_target_path()` — Settings and Keymap panes each expose `fn editor_target(&self) -> Option<&Path>`; framework chooses based on which is focused.

---

## Phases

Each phase is a single mergeable commit. Each commit must build green and pass `cargo nextest run`. No sub-phases (`Na/Nb/Nc`) — every increment gets its own integer.

### Phase 1 — Workspace conversion ✅

Convert `cargo-port-api-fix` into a Cargo workspace. See `phase-01-cargo-mechanics-DONE.md` for full TOML and `phase-01-ci-tooling-DONE.md` for the CI updates.

Concrete steps:

1. Root `Cargo.toml` keeps `[package]` (binary) and adds `[workspace] members = ["tui_pane"]` + `resolver = "3"` (resolver must be explicit; not inferred from edition 2024 in workspace context).
2. Promote the existing `[lints.clippy]` and `[lints.rust]` blocks verbatim to `[workspace.lints.clippy]` / `[workspace.lints.rust]` (including `missing_docs = "deny"` from day one). Root `[lints]` becomes `workspace = true`.
3. Create `tui_pane/` as a sibling directory (not `crates/tui_pane/`) with `Cargo.toml` (`crossterm`, `ratatui`, `dirs` deps; `[lints] workspace = true`) and `src/lib.rs` carrying crate-level rustdoc.
4. Add `tui_pane = { path = "tui_pane", version = "0.0.4-dev" }` to the binary's `[dependencies]`.
5. Apply the CI flag updates flagged in `phase-01-ci-tooling-DONE.md` §2 (e.g. `cargo +nightly fmt --all`, `cargo mend --workspace --all-targets`, `cargo check --workspace` in the post-tool-use hook). Per the spec these can ship in a separate prior commit since they're no-ops on the current single-crate layout.
6. Update auto-memory `feedback_cargo_nextest.md` to clarify default `cargo nextest run` only tests the root package; iteration loops should pass `-p` or `--workspace`. `feedback_cargo_install.md` is unchanged (the binary stays at root).

After Phase 1: `cargo build` from the root builds both crates; `cargo install --path .` still installs the binary; `Cargo.lock` and `target/` stay at the workspace root.

**Per-phase rustdoc precondition.** Phases 2–17 add `pub` items to `tui_pane`. Each pub item ships with a rustdoc summary line — `missing_docs = "deny"` is workspace-wide from Phase 1, so a missing doc breaks the build. Module headers (`//!` blocks) must use the format **one-line summary, blank `//!`, then body** — `clippy::too_long_first_doc_paragraph` (nursery) rejects multi-sentence opening paragraphs (Phase 3 retrospective surfaced this).

### Phases 2–10 — `tui_pane` foundations

Phases 2–10 land the entire `tui_pane` public surface in dependency order, one mergeable commit per phase. The canonical type spec is `core-api.md` (sections referenced from each sub-phase below); §11 is the canonical module hierarchy and the public re-export set in `lib.rs`. Type detail per file is in `core-api.md` §§1–10.

**Strictly additive across Phases 2–10.** Nothing moves out of the binary in this group. The binary continues to use its in-tree `keymap_state::Keymap`, `shortcuts::*`, etc., untouched. The migration starts in Phase 13.

**Pre-Phase-2 precondition (post-tool-use hook).** Decide hook strategy before Phase 2 lands: repo-local override at `.claude/scripts/hooks/post-tool-use-cargo-check.sh` adding `--workspace`, vs. updating the global script at `~/.claude/scripts/hooks/post-tool-use-cargo-check.sh`. Without the flag, edits to `tui_pane/src/*.rs` from inside the binary working dir will not surface `tui_pane` errors. Repo-local override is the lower-blast-radius option.

**README precondition (Phase 10).** `tui_pane/README.md` lands at the end of Phase 10 — when the public API is complete. It covers crate purpose + a minimal example using `Framework::new(initial_focus)`. Code blocks in the README are ` ```ignore ` (no doctests in this crate).

### Phase 2 — Keys ✅

Add `tui_pane/src/keymap/key_bind.rs` (`KeyBind`, `KeyInput`, `KeyParseError`) per `core-api.md` §1. Leaf types — nothing else in `tui_pane` depends on them yet.

Construction surface for `KeyBind`: `From<KeyCode>`, `From<char>` (modifier-free); `KeyBind::shift(impl Into<Self>)`, `KeyBind::ctrl(impl Into<Self>)` (modifier-bearing, OR-composable). **No** `From<KeyEvent>` — the kind discriminant must not be silently dropped.

`KeyInput` is the event-loop-facing enum: `Press(KeyBind) | Release(KeyBind) | Repeat(KeyBind)` produced by `KeyInput::from_event(KeyEvent)`. Keymap dispatch only handles `Press` (use `.press() -> Option<&KeyBind>`); Release / Repeat flow through for any future opt-in handler. Modeled after Zed/GPUI's `KeyDownEvent`/`KeyUpEvent` type split.

`KeyParseError` is `#[derive(thiserror::Error)]` (Phase 1 added `thiserror = "2"` to `tui_pane/Cargo.toml`). Every error type added to `tui_pane` in later phases (`KeymapError`, etc.) follows the same pattern.

Unit tests:
- `KeyBind::parse` accepts `"Enter"`, `"Ctrl+K"`, `"Shift+Tab"`, `"+"`, `"="`; the pre-refactor `+`/`=` collapse is dropped (they parse to distinct `KeyCode::Char` values).
- `display_short` walks every `KeyCode` variant the parser can produce and asserts the result never contains `,` or `/` (paired-slot constraint enforced by a `debug_assert!` in `Paired` rendering).

#### Retrospective

**What worked:**
- `From<KeyCode>` / `From<char>` collapsed cleanly — no need for named `plain` / `from_event` methods on `KeyBind`. The `KeyEvent` bridge moved to a separate kind-tagged `KeyInput` enum (`Press`/`Release`/`Repeat`) rather than a lossy `From<KeyEvent>` impl, so keymap dispatch can pattern-match on `Press` and never accidentally fire on `Release`.
- `thiserror::Error` derive + `#[error("...")]` per variant is shorter than hand-written `Display`/`Error` and gives free `#[from]`/`#[source]` chaining for downstream wrappers.
- 10 unit tests pass; clippy clean under workspace pedantic+nursery+all lint stack.

**What deviated from the plan:**
- `KeyBind::shift` / `ctrl` were respec'd to take `impl Into<Self>` (i.e. `impl Into<KeyBind>`) rather than `impl Into<KeyCode>`. Reason: crossterm's `KeyCode` does not implement `From<char>`, so the planned `impl Into<KeyCode>` bound rejects `KeyBind::shift('g')`. Taking `Into<KeyBind>` reuses the three `From` impls and makes `shift`/`ctrl` composable (`KeyBind::ctrl(KeyBind::shift('g'))` → CTRL|SHIFT). `core-api.md` §1 still lists the old signature.
- `KeyParseError` ships with 3 variants (`Empty`, `UnknownKey`, `UnknownModifier`) — `InvalidChar` from the spec was dropped because no parser path emits it. `core-api.md` §1 still lists 4 variants.
- Parser supports `"Control"` as a synonym for `"Ctrl"` (both produce `KeyModifiers::CONTROL`); `"Space"` parses to `KeyCode::Char(' ')`. Neither was called out in the plan.

**Surprises:**
- `KeyCode` has no `From<char>` impl in crossterm — and orphan rules block adding one. This forced the `impl Into<Self>` rework.
- Modifier display order (`Ctrl` → `Alt` → `Shift`) and the case-preservation policy in `parse` (`"Ctrl+K"` → `Char('K')`, not `Char('k')`) are now baked into Phase 2 tests. Phase 9 (TOML loader) inherits both as facts; if the loader needs case-insensitive letter lookup, that is a *keymap-layer* normalization, not a `KeyBind::parse` concern.

**Implications for remaining phases:**
- `core-api.md` §1 is out of sync with shipped code (signatures + error variants). Update before any later phase reads it as canonical.
- Phase 9 (`Keymap<Ctx>` + TOML loader) must decide letter-case normalization policy explicitly — `parse` preserves case as-is.
- Future framework error types (`KeymapError` Phase 4 skeleton, fill in Phase 9) should use `#[derive(thiserror::Error)]` with `#[from] KeyParseError` for source chaining, per the pattern established here.

#### Phase 2 Review

- Phase 3: rename `keymap/traits.rs` → `keymap/action_enum.rs` so the file name matches its sole resident (`Action` + `action_enum!`) and does not collide with Phase 7's per-trait file split.
- Phase 4: `KeymapError` ships with `#[derive(thiserror::Error)]` + `#[from] KeyParseError` for source chaining, and unit tests are rescoped to constructs that exist by end of Phase 4 (vim-application test deferred to Phase 10). `bindings!` macro tests now cover composed `KeyBind::ctrl(KeyBind::shift('g'))`.
- Phase 9: loader explicitly lowercases single-letter TOML keys (so `quit = "Q"` binds `Char('q')`); modifier display order is canonical `Ctrl+Alt+Shift+key` (no round-trip ordering preservation); vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods); `KeymapError` source chain from `KeyParseError` is asserted.
- Phase 12: paired-row separator policy made explicit — `Paired::debug_assert!` covers only the parser-producible `KeyCode` set; exotic variants may panic, and widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep.
- Phase 13: `anyhow = "1"` lands in the binary's `Cargo.toml` here (first call site that needs context wrapping is `Keymap::<App>::builder(...).load_toml(?)?.build()?`).
- §1 (`Pane id design`): `PaneId` → `FrameworkPaneId` everywhere, including the inside-the-crate short form, so the type name is one-to-one across library and binary call sites.
- `core-api.md` §1 + `tui-pane-lib.md` §11 lib.rs sketch synced to shipped Phase 2 code: `shift`/`ctrl` take `impl Into<Self>`, `From<KeyEvent>` documented, 3-variant `KeyParseError` (`InvalidChar` dropped), parser policy (`"Control"` synonym, `"Space"` token, case-preserving) called out.
- `toml-keys.md` synced to the Zed/VSCode/Helix-aligned letter-case decision: loader lowercases single-letter ASCII keys (`"Q"` → `Char('q')`, never `Shift+q`); modifier tokens are case-insensitive on input but writeback canonical capitalized; named-key tokens (`Enter`, `Tab`, `F1`, …) are case-sensitive with no aliases; non-ASCII letters not lowercased; modifier repeats silently OR'd (not rejected — bitwise OR is idempotent).
- Phase 6 + Phase 11 now spell out the **Phase 6 → Phase 11 contract**: Phase 6 freezes a 1-field / 3-method `Framework<Ctx>` skeleton (`focused` field, `new`/`focused()`/`set_focused()`); Phase 11 is purely additive on top. Mirrored at both phase blocks so neither side can drift independently.
- Decided: `KeyEvent` press/release/repeat handling uses a typed wrapper enum (`KeyInput { Press, Release, Repeat }`) at the framework boundary, not a runtime check at each dispatch site and not a fallible `Option`-returning conversion. Modeled after Zed/GPUI's typed-event split. Repeat is preserved (not collapsed into Press) so future handlers can opt into auto-repeat behavior. Phases 13–15 dispatch sites pattern-match `KeyInput::Press(bind)` (or call `.press()`); the event-loop entry produces `KeyInput` once.

### Phase 3 — Action machinery ✅

Add `tui_pane/src/keymap/action_enum.rs` with `Action` + `action_enum!` (per §4 — the trait part; the three scope traits land in Phase 7). Add `tui_pane/src/keymap/global_action.rs` with `GlobalAction` and its `Action` impl (§10). Add `tui_pane/src/keymap/vim.rs` with `VimMode::{Disabled, Enabled}` (§10).

> File `action_enum.rs` (not `traits.rs`) and `global_action.rs` (not `base_globals.rs`) — the file name matches the contained type. The three scope traits live in their own files (`shortcuts.rs` / `navigation.rs` / `globals.rs`) per Phase 7.

#### Retrospective

**What worked:**
- Three-file split (`action_enum.rs` / `global_action.rs` / `vim.rs`) lined up one-to-one with shipped code — no scope drift. 12 unit tests cover macro expansion (`action_enum!` against a fixture `Foo` enum) and the hand-rolled `GlobalAction` impl. Workspace clippy clean under `pedantic` + `nursery` + `all` + `cargo`.
- `pub use keymap::Action;` at crate root in `lib.rs` keeps the macro's `$crate::Action` path stable regardless of the trait's true module location. The macro can be re-homed later without breaking any expansion site.
- `VimMode` defaults to `Disabled` via `#[derive(Default)]` + `#[default]` on the variant — no hand-written `Default` impl needed.

**What deviated from the plan:**
- Hand-rolled `impl Display for GlobalAction` (delegates to `description()`) — not strictly required by the spec but mirrors what the macro generates for `action_enum!`-produced enums, so all `Action` impls render the same way under `format!("{action}")`. Cost: 4 lines.
- `crate::Action` (root re-export) is the trait path used inside `global_action.rs`'s test module rather than a longer `super::super::action_enum::Action` — single-`super::` is fine in normal code, double-`super::` is banned by project policy.

**Surprises:**
- `clippy::too_long_first_doc_paragraph` (nursery) fires on multi-sentence module headers. `global_action.rs`'s opening `//!` block had to be split into a one-line summary + blank `//!` + body. Likely to fire elsewhere when later phases ship docs. No code change required, but worth knowing for module-doc authoring.
- The `from_toml_key` returning `Option<Self>` (not `Result`) is intentional and the trait method has no scope context to attach. The TOML loader (Phase 4 skeleton, Phase 9 fill) lifts `None` into `KeymapError::UnknownAction { scope, action }`. Recorded explicitly here so Phase 4/8 don't accidentally widen the trait.

**Implications for remaining phases:**
- `macros.md` §3 still references `crate::keymap::traits::Action` — needs to be `crate::keymap::action_enum::Action`. Synced as part of this review.
- `core-api.md` §10 `KeymapError` snippet has `impl Display { /* … */ }` placeholder — Phase 4 lands the real impl via `#[derive(thiserror::Error)]` per the Phase 2 retrospective decision.
- Phase 4 (`bindings!` macro) follows the same `#[macro_export] macro_rules!` declaration template used here; the doctest pattern can mirror Phase 3's approach (`crate::action_enum! { … }` inside an internal `mod tests`).
- Phase 13 (binary swap to `tui_pane::action_enum!`): seven existing `action_enum!` invocations in `src/keymap.rs` swap to the `tui_pane::` prefix; the macro's grammar is identical, so each invocation needs only the prefix change.

#### Phase 3 Review

Architectural review of remaining phases (4-17) returned 18 findings — 13 minor (applied directly), 5 significant (decided with the user). Resolved outcomes:

- **Renamed `keymap/base_globals.rs` → `keymap/global_action.rs`** so the file name matches the contained type (`GlobalAction`). User did the file rename in their editor; doc references and `mod.rs` synced. No `BaseGlobals` type ever existed; the "base" prefix earned nothing and broke the established `key_bind.rs → KeyBind` convention.
- **Phase 9 anchor type:** `Keymap<Ctx>` lives in `keymap/mod.rs` (option c). Workspace lint `self_named_module_files = "deny"` rules out `keymap.rs` + `keymap/` sibling layout, and `clippy::module_inception` rules out `keymap/keymap.rs`. Phase 6 already follows the same convention with `framework/mod.rs` holding `Framework<Ctx>`. Plan's prior `keymap/mod_.rs` was a typo.
- **Framework owns `GlobalAction` dispatch (significant pivot, item 2):** `KeymapBuilder` no longer takes positional `(quit, restart, dismiss)` callbacks. Framework dispatches all seven variants:
  - `Quit` / `Restart` set `Framework<Ctx>::quit_requested` / `restart_requested` flags; binary's main loop polls.
  - `Dismiss` runs framework chain (toasts → focused framework overlay), then bubbles to optional `dismiss_fallback`.
  - `NextPane` / `PrevPane` / `OpenKeymap` / `OpenSettings` framework-internal as before.
  - Binary opts in via optional `.on_quit()` / `.on_restart()` / `.dismiss_fallback()` chained methods on `KeymapBuilder`.
  - Rationale: hit-test for the mouse close-X on framework overlays already lives in the framework. Splitting Esc-key dismiss between framework (overlays) and binary (everything else) duplicates that ownership.
  - Touches Phase 6 (Framework skeleton +2 fields, +2 methods), Phase 10 (KeymapBuilder drops 3 args, gains 3 chained hooks), Phase 11 (Toasts dismiss participation, `Framework::dismiss()` method), Phase 17 (binary main loop polls flags, deletes `Overlays::should_quit`).
- **Cross-enum `[global]` collision = hard error (item 3):** `KeymapError::CrossEnumCollision { key, framework_action, app_action }` at load time. Definition-time error — app dev renames their colliding `AppGlobalAction::toml_key` string. Per-binding revert policy still handles user typos.
- **`GlobalAction::defaults()` lives on the enum (item 4):** `pub fn defaults() -> Bindings<Self>` lands in Phase 4 (when `Bindings` + `bindings!` exist) inside `global_action.rs`. Loader and builder consume it.
- **Cross-crate macro integration test (item 5):** `tui_pane/tests/macro_use.rs` lands as a Phase 3 follow-up — exercises `tui_pane::action_enum!` from outside the crate. Phase 4 extends it for `tui_pane::bindings!`.

Minor findings applied directly (no user gating):
- Stale `keymap::traits::Action` references in `macros.md` synced to `keymap::action_enum::Action`.
- Phase 4 root re-exports (`Bindings`, `KeyBind`) called out for the `bindings!` macro's `$crate::` paths.
- `KeymapError` variant set spelled out in Phase 4 + `core-api.md` §10 (with `#[derive(thiserror::Error)]`).
- `Shortcuts::vim_extras() -> &'static [(Self::Actions, KeyBind)]` called out in Phase 7 with default `&[]`.
- Vim-mode skip-already-bound test moved Phase 9 → Phase 10 (vim application is the builder's job per "Vim mode — framework capability" §).
- `AppContext::AppPaneId: Copy + Eq + Hash + 'static` super-trait set added to Phase 6 (required by Phase 11's `HashMap<AppPaneId, fn(&Ctx) -> Mode<Ctx>>`).
- `core-api.md` §4 `Action` super-trait set synced to shipped code (`Copy + Eq + Hash + Debug + Display + 'static` — adds `Debug` + `Display`).
- Phase 9 explicit "loader lifts `None` from `from_toml_key` into `KeymapError::UnknownAction`" wording added.
- `clippy::too_long_first_doc_paragraph` (nursery) guidance added to the per-phase rustdoc precondition.
- `pub use keymap::GlobalAction;` at crate root noted in Phase 13.
- Paired-row separator policy in Phase 12 shortened to a one-line cross-reference of Phase 2's locked decision.

### Phase 4 — Bindings, scope map, loader errors ✅

Add `tui_pane/src/keymap/bindings.rs` (`Bindings<A>` + `bindings!`, §2), `tui_pane/src/keymap/scope_map.rs` (`ScopeMap<A>`, §3), and `tui_pane/src/keymap/load.rs` skeleton holding `KeymapError` (§10). The loader's actual TOML-parsing impl lands in Phase 9 alongside `Keymap<Ctx>`.

**Also lands in Phase 4 (post-Phase-3 review):** `pub fn defaults() -> Bindings<Self>` on `GlobalAction` in `tui_pane/src/keymap/global_action.rs` — returns the canonical `q` / `R` / `Tab` / `Shift+Tab` / `Ctrl+K` / `s` / `x` bindings using the `bindings!` macro that ships in this phase. Co-located with the enum (matches the convention every `Shortcuts<P>::defaults()` impl follows). Tested in `global_action.rs` directly; loader and builder consume it.

**Root re-exports.** `tui_pane/src/lib.rs` is `mod keymap;` (private) plus crate-root `pub use` for every public type: `Action`, `Bindings`, `GlobalAction`, `KeyBind`, `KeyInput`, `KeyParseError`, `KeymapError`, `ScopeMap`, `VimMode`. The `bindings!` macro lands at the crate root via `#[macro_export]`, so no explicit re-export is needed.

`KeymapError` is `#[derive(thiserror::Error)]` and ships with seven variants (the loader and builder consume them in Phases 8 and 9):
- `Io(#[from] std::io::Error)` — file-open failure.
- `Parse(#[from] toml::de::Error)` — top-level TOML parse failure.
- `InArrayDuplicate { scope, action, key }` — duplicate key inside one TOML array.
- `CrossActionCollision { scope, key, actions: (String, String) }` — same key bound to two actions.
- `InvalidBinding { scope, action, #[source] source: KeyParseError }` — `KeyBind::parse` failure with chained source.
- `UnknownAction { scope, action }` — `A::from_toml_key(key)` returned `None`; loader attaches the scope.
- `UnknownScope { scope }` — TOML referenced an unknown top-level table.

Phase 4 ships the `enum` definition; Phase 9 wires the actual loader paths that emit each variant. Phase 10 adds three more variants (`NavigationMissing`, `GlobalsMissing`, `DuplicateScope`) so `KeymapError` covers builder validation too — `BuilderError` was rejected during Phase 9 review (one error type, not two).

`bindings!` macro grammar must accept arbitrary `impl Into<KeyBind>` expressions on the RHS — including composed forms like `KeyBind::ctrl(KeyBind::shift('g'))` (CTRL|SHIFT, established by Phase 2). The macro's unit tests cover the composed case.

**Cross-crate macro integration test.** Extend `tui_pane/tests/macro_use.rs` (the scaffolding lands as a Phase 3 follow-up exercising `action_enum!` only) to add a `bindings!` invocation. Both macros are compiled here from outside the defining crate — `#[macro_export]` + `$crate::` paths are easy to break under cross-crate use, and this test locks the public path before Phase 13's binary swap depends on it.

Unit tests (this phase, scoped to what exists by end of Phase 4):
- `Bindings::insert` preserves insertion order; first key for an action is the primary.
- `ScopeMap::add_bindings` on an empty map produces `by_key.len() == by_action.values().map(Vec::len).sum::<usize>()` (no orphan entries).
- `bindings!` accepts `KeyBind::ctrl(KeyBind::shift('g'))` and stores `KeyModifiers::CONTROL | SHIFT`.
- (Deferred to Phase 10, when the builder + `VimMode::Enabled` application pipeline exist:) `MockNavigation::Up` keeps its primary as the inserted `KeyCode::Up` even with `VimMode::Enabled` applied — insertion-order primary.

#### Retrospective

**What worked:**
- `bindings!` macro grammar (`KEY => ACTION` and `[KEYS] => ACTION` arms with optional trailing commas) accepted every authoring case the test suite threw at it, including `KeyBind::ctrl(KeyBind::shift('g'))` composed modifiers.
- `tests/macro_use.rs` cross-crate test caught a `$crate::*` path break the moment we flipped `pub mod keymap` → `mod keymap` (cross-crate paths started failing immediately, before any consumer noticed).
- 49 tui_pane tests pass; 599 workspace tests pass; `cargo mend --fail-on-warn` reports no findings.

**What deviated from the plan:**
- **`pub mod` removed everywhere.** Plan said "extend root re-exports for `Bindings`, `KeyBind`." Per `cargo mend` (which denies `pub mod` workspace-wide) and direct user instruction, `tui_pane/src/lib.rs` was reduced to `mod keymap;` (private) plus crate-root `pub use` for every public type: `Action`, `Bindings`, `GlobalAction`, `KeyBind`, `KeyInput`, `KeyParseError`, `KeymapError`, `ScopeMap`, `VimMode`. `keymap/mod.rs` similarly switched all `pub mod foo;` to `mod foo;` + facade `pub use`. **Public-API change:** the `tui_pane::keymap::*` namespace no longer exists — every type is now flat at `tui_pane::*`.
- **`bindings!` macro is a two-step expansion.** Spec'd as a single `macro_rules!` with one block-returning arm. A single arm cannot recurse to handle mixed `KEY => ACTION` / `[KEYS] => ACTION` lines, so the macro now delegates to a `#[doc(hidden)] #[macro_export] macro_rules! __bindings_arms!` incremental TT muncher. Public surface unchanged; `__bindings_arms!` is the implementation detail.
- **`ScopeMap::new` / `insert` are `pub(super)`, not `pub(crate)`.** The design doc said `pub(crate)`; project memory `feedback_no_pub_crate.md` (use `pub(super)` in nested modules — `pub(crate)` reserved for top-level files) overruled. Same author intent (framework-only construction), narrower scope.
- **`bind_many` requires `A: Clone`, not just `A: Copy`.** The loop body needs to clone the action per key; `Copy` only matters when the entire `Bindings` is consumed. Trivial in practice — every `Action` is `Copy + Clone`.
- **`bindings!` uses `$crate::KeyBind`, not `$crate::keymap::KeyBind`.** Falls out of the `pub mod keymap` removal: the macro's `$crate::*` paths now reach the flat root re-exports.

**Surprises:**
- **clippy `must_use_candidate` (pedantic) fires on every getter.** Each new public method that returns a value needs `#[must_use]`. Apply pre-emptively in Phase 5+.
- **`cargo mend` denies `pub mod` workspace-wide and there is no `mend.toml` allowlist.** Phases 5–11 must declare every new module as private `mod foo;` plus `pub use foo::Type;` at the parent facade — never `pub mod foo;`.
- **`src/tui/panes/support.rs` had three pre-existing mend warnings** (inline path-qualified types) that auto-resolved during the Phase 4 build cycle — picked up "for free." Not part of Phase 4 scope but landed in the same diff.

**Implications for remaining phases:**
- **Every Phase 5+ module declaration must be `mod foo;`** (not `pub mod foo;`) at every level. Affects Phase 5 (`bar/region.rs`, `bar/slot.rs`), Phase 6 (`framework/`), Phase 7 (scope traits), Phase 9 (`keymap/container.rs` or wherever `Keymap<Ctx>` lands), Phase 10 (`keymap/builder.rs`), Phase 11 (`panes/*`), Phase 12 (`bar/render.rs`).
- **Every `tui_pane::keymap::*` path in design docs is now stale.** `phase-02-core-api.md`, `phase-02-macros.md`, `phase-02-test-infra.md`, `phase-02-toml-keys.md`, and the rest of `tui-pane-lib.md` need a sweep: `crate::keymap::Foo` → `crate::Foo` (and `tui_pane::keymap::Foo` → `tui_pane::Foo` in public-API examples).
- **Phase 13 binary swap uses flat paths.** `use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`. Every file in `src/tui/` that touches keymap types will see this.
- **`pub(super)` is the visibility default for framework-internal construction.** Phase 9's `Keymap<Ctx>` constructor, Phase 10's `KeymapBuilder::build()` — apply the same rule: `pub(super)` for sites only the framework's own `keymap/` siblings call.
- **Pre-emptive `#[must_use]` on every Phase 5+ public getter** saves a clippy round-trip per phase.

#### Phase 4 Review

- **Phase 4 plan text reconciled** with shipped `KeymapError` (added `Io(#[from])` and `Parse(#[from] toml::de::Error)` — the previous variant list of 5 omitted them).
- **Stale "Extend root re-exports" paragraph rewritten** to reflect the shipped lib.rs (every public type re-exported flat at crate root; no `pub use keymap::bindings::bindings;`).
- **Phase 5 (Bar primitives)** gains an explicit "Root re-exports" line: `lib.rs` adds `pub use bar::{BarRegion, BarSlot, Mode, ShortcutState, Visibility};`. The `Shortcut` wrapping struct is gone — `Shortcuts::visibility` returns `Visibility` and `Shortcuts::state` returns `ShortcutState`; the bar label is `Action::bar_label()` (no per-frame override).
- **Phase 6 (Framework skeleton)** gains an explicit "Root re-exports" line: `pub use framework::Framework;`, `pub use pane_id::{FocusedPane, FrameworkPaneId};`, `pub use app_context::AppContext;` plus `#[must_use]` directive.
- **Phase 7 (Scope traits)** gains: `pub use keymap::{Shortcuts, Navigation, Globals};` plus standing-rule 1 reminder.
- **Phase 9 (Keymap container)** gains: `pub use keymap::Keymap;`, `pub(super)` for `Keymap::new`, `#[must_use]` on getters.
- **Phase 10 (Keymap builder)** gains: `pub use settings::SettingsRegistry;`, `pub(super)` for builder internals. `KeymapBuilder` and `KeymapError` are already re-exported from Phase 9; no `BuilderError` to add (one-error-type decision).
- **Phase 11 (Framework panes)** gains: `pub use panes::{KeymapPane, SettingsPane, Toasts};`, panes/mod.rs declared `mod` (private) per standing rule 1.
- **Phase 12 (Bar renderer)** gains: `pub use bar::StatusBar;` plus standing-rule 1 reminder.
- **Phase 13 (App swap)** gains: flat-namespace import note (`use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`) and binary-side `mod` rule reminder.
- **New "Phase 5+ standing rules" subsection** added after the Phase 4 retrospective: locks the seven standing rules (private `mod`, flat re-exports, `pub(super)` for framework-internal, `#[must_use]` on getters, flat `$crate::*` macro paths, new `#[macro_export]` extends `tests/macro_use.rs`, `cargo mend --fail-on-warn` as phase-completion gate).
- **Definition of done** rewritten to enumerate every public type at crate-root flat paths and to call out `__bindings_arms!` as `#[doc(hidden)]` but technically reachable.
- **Spec docs swept** (`core-api.md`, `macros.md`): stale `crate::keymap::<submod>::Foo` sub-paths replaced with the facade-path form `crate::keymap::{Foo, ...}`; explanatory comments added about why the public API is flat.
- **Reviewed and not changed:** `tui_pane/README.md` deferred to Phase 17 (subagent finding #20 — no earlier baseline justified). `bind_many` requiring `A: Clone` (subagent finding #10 — auto-satisfied because `Action: Copy`, no plan change needed).

These apply to every remaining phase without further mention; phase blocks below assume them. Restate only where a phase has a specific exception.

1. **Module declarations are `mod foo;`** at every level — never `pub mod foo;`. Parents expose the API via `pub use foo::Type;` re-exports. `cargo mend` denies `pub mod` workspace-wide, including the binary side (`src/tui/...` in Phase 13).
2. **Public types live at the crate root.** Every `tui_pane` public type re-exports from `tui_pane/src/lib.rs` so callers write `tui_pane::Foo` (flat). The `tui_pane::keymap::*` namespace does not exist publicly.
3. **Framework-internal construction is `pub(super)`.** New / insert / build methods that only the framework's own siblings call use `pub(super)`, never `pub(crate)`. Project memory `feedback_no_pub_crate.md` for rationale.
4. **Public getters get `#[must_use]` pre-emptively.** Clippy `must_use_candidate` (pedantic, denied) fires on every getter that returns a value the caller can ignore.
5. **Macros use flat `$crate::*` paths.** Every `#[macro_export]` macro references re-exported root types: `$crate::Bindings`, `$crate::KeyBind`, `$crate::Action`. Never `$crate::keymap::Foo`.
6. **New `#[macro_export]` extends `tests/macro_use.rs`.** Cross-crate path stability is locked by that file; any new exported macro adds an invocation there.
7. **Phase-completion gates.** `cargo build`, `cargo nextest run`, `cargo +nightly fmt`, `cargo clippy --workspace --all-targets`, `cargo mend --fail-on-warn` — all clean before the phase is marked ✅.
8. **Every new pub item gets a doc comment; every new module gets a `//!` header.** Module `//!` explains what lives in the file and why; type `///` explains the role; method `///` explains what callers get back; variant `///` explains the case. One-liners are fine where the name carries the meaning. The Phase 5 files (`bar/region.rs`, `bar/slot.rs`, `bar/mod.rs`) and Phase 3's `keymap/action_enum.rs` / `keymap/global_action.rs` are the reference baseline — match that density.
9. **Public `&self` value-returning methods carry both `#[must_use]` and `const fn`.** Setters (`&mut self`) carry `const fn` when the body is const-eligible (Rust 1.83+ permits `&mut` in const fn). Clippy nursery `missing_const_for_fn` is denied workspace-wide and fires on every getter / setter that could be const. Phase 6's `Framework<Ctx>` (5 methods, all `const fn`) is the reference baseline.

### Phase 5 — Bar primitives ✅

Add `tui_pane/src/bar/region.rs` (`BarRegion::{Nav, PaneAction, Global}` + `ALL`), `tui_pane/src/bar/slot.rs` (`BarSlot<A>` + `ShortcutState` + `Visibility`), and `Mode<Ctx>` in `bar/mod.rs`. All per §5.

Phase 5 also amends Phase 3's `Action` trait to add `fn bar_label(self) -> &'static str` and extends the `action_enum!` macro grammar to take a tuple of three string literals per arm:

```rust
action_enum! {
    pub enum PackageAction {
        Activate => ("activate", "activate", "Open / activate selected field");
        Clean    => ("clean",    "clean",    "Clean target dir");
        //          ↑ TOML key   ↑ bar label ↑ keymap-UI description
    }
}
```

Leaf types only — the renderer that consumes them lands in Phase 12.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::{BarRegion, BarSlot, Mode, ShortcutState, Visibility};`. `bar/mod.rs` is `mod region; mod slot; pub use region::BarRegion; pub use slot::{BarSlot, ShortcutState, Visibility}; pub use ...Mode;` (or wherever `Mode` lands).

**No `Shortcut` wrapping struct.** Phase 7's `Shortcuts<Ctx>` trait splits the bar-entry payload across two orthogonal axes: `fn visibility(&self, action, ctx) -> Visibility` (default `Visible`, `Hidden` removes the slot) and `fn state(&self, action, ctx) -> ShortcutState` (default `Enabled`, `Disabled` grays the slot). The label is static (`Action::bar_label()`); there is no per-frame label override.

**`action_enum!` grammar amendment.** The macro arm changes from `Variant => "key", "desc";` to `Variant => ("key", "bar", "desc");`. Phase 3's existing `action_enum!` invocations in the keymap module and the `tests/macro_use.rs` smoke test must be updated in this phase. The 12-arm cargo-port migration in Phase 13 inherits the new grammar. The hand-rolled `GlobalAction` `Action` impl shipped in Phase 3 also needs a `bar_label()` method body — one match arm per variant (`Quit => "quit"`, `Restart => "restart"`, etc.).

**`Globals::bar_label` removed.** With `Action::bar_label` available on every action enum, the redundant `fn bar_label(action: Self::Actions) -> &'static str` method on the `Globals<Ctx>` trait is not present. Bar code calls `action.bar_label()` regardless of which scope the action came from.

**No pre-existing call sites for `Shortcuts::visibility` / `state`.** The `Shortcuts<Ctx>` trait itself lands in Phase 7, so Phase 5 has nothing to migrate beyond the `action_enum!` arms. `tests/macro_use.rs` extends with a smoke test constructing `tui_pane::BarSlot::Single(...)`, `tui_pane::BarRegion::Nav`, `tui_pane::ShortcutState::Enabled`, and `tui_pane::Visibility::Visible` from outside the crate to lock the flat-namespace public path.

#### Retrospective

**What worked:**
- `bar/region.rs`, `bar/slot.rs`, `bar/mod.rs` landed as flat `mod`-private files with crate-root re-exports — standing rules 1 + 2 applied without friction.
- Macro grammar change to `Variant => ("toml_key", "bar_label", "description");` was a single `macro_rules!` arm edit; both the inline `Foo` test enum and `tests/macro_use.rs` migrated trivially.
- Cross-crate test (`bar_primitives_reachable_from_outside_crate`) caught the public path before any consumer needed it — `tui_pane::BarSlot::Single`, `tui_pane::BarRegion::ALL`, `tui_pane::ShortcutState::Enabled`, `tui_pane::Mode::Navigable`, `tui_pane::Visibility::Visible` all reachable.
- 59 tui_pane tests pass; 659 workspace tests pass; clippy + mend clean.

**What deviated from the plan:**
- **Doc backticks needed on `BarRegion` variant references in `Mode` docstrings.** Pedantic clippy `doc_markdown` flagged `PaneAction` mid-doc; wrapped `Nav`/`PaneAction`/`Global` in backticks. Standing rule 4 (`#[must_use]`) is the per-getter form of this same broader pedantic-clippy posture; bar primitives have no getters, so #4 didn't apply this phase.
- **`GlobalAction::bar_label` strings chosen explicitly.** Plan said "match arms per variant (`Quit => "quit"`, etc.)" without committing the full set. Shipped: `quit`, `restart`, `next`, `prev`, `keymap`, `settings`, `dismiss` — short forms for `NextPane`/`PrevPane`/`OpenKeymap`/`OpenSettings` (the `Open` prefix and `Pane` suffix are bar noise).

**Surprises:**
- **`bar_label` shorter than `toml_key` for `GlobalAction`.** Pattern: `toml_key = "open_keymap"`, `bar_label = "keymap"`, `description = "Open keymap viewer"`. Three-axis labelling (config-stable / bar-terse / human-readable) is the value the macro grammar buys us; the example arms in the plan all happened to use identical `toml_key`/`bar_label`, masking this.

**Implications for remaining phases:**
- **Phase 7 bar label is static.** `Shortcuts<Ctx>` has no `label` method — the bar label for an action is always `Action::bar_label()` (declared in `action_enum!`). Per-frame visibility goes through `Shortcuts::visibility(action, ctx) -> Visibility { Visible | Hidden }`.
- **Phase 7 `Shortcuts::state` default is `ShortcutState::Enabled`.** Same: zero per-impl boilerplate.
- **Phase 13 cargo-port `action_enum!` migrations need the third positional string.** Every existing app-side invocation gains a bar label between the toml key and description. For app actions where the bar text matches the toml key, just duplicate the literal — no design decision per arm.
- **Phase 12 bar renderer reads `BarRegion::ALL` for layout order.** Already reflected in trait def — `Vec<(BarRegion, BarSlot<Self::Actions>)>` returned, renderer groups by region.
- **No new public types added to `tui_pane::*` beyond the announced bar primitives** (`BarRegion`, `BarSlot`, `Mode`, `ShortcutState`, `Visibility`). Every later-phase reference to `tui_pane::Shortcut` (the deleted wrapping struct) is dead — caught any in Phase 5's plan-doc sweep, but Phase 7 implementers should not pattern-match on `Shortcut` in muscle memory.

#### Phase 5 Review

- **Phase 7 (Scope traits)** plan body now enumerates the full `Shortcuts<Ctx>` method set (cross-references `core-api.md` §4) and explicitly states the `label` / `state` default bodies leveraging `Action::bar_label` and `ShortcutState::Enabled`.
- **Phase 7** also explicitly states `Globals<Ctx>` has no `bar_label` method, and adds a `Shortcut` (singular wrapping struct) doc-grep step to confirm zero residue.
- **Phase 9 (Keymap container)** plan gains a one-line clarification that `bar_label` is code-side only — the TOML loader never reads or writes it.
- **Phase 12 (Bar renderer)** plan now states the per-region `Mode` suppression rules in line with shipped `bar/mod.rs` docstrings (`Static` suppresses `Nav`, `TextInput(_)` suppresses `Nav` + `PaneAction` + `Global`).
- **Phase 13 (App swap)** gains an explicit migration-cost callout that every existing `action_enum!` invocation in `src/tui/` needs a third positional `bar_label` literal.
- **Phase 18 (Regression tests)** reworded to assert each global slot's bar text comes from `action.bar_label()`, not a `Globals` trait method.
- **Doc-spec sync (`core-api.md`):** `ScopeMap::new`/`insert` migrated from `pub(crate)` → `pub(super)` to match shipped code (Phase 4 retrospective decision; finalized here per post-phase doc-sync rule).
- **Reviewed and not changed:** `Globals::render_order` (subagent finding #6 — already declared at `core-api.md:423`, plan unchanged); binary-side `pub mod` audit in Phase 13 (subagent finding #11 — grep of `src/tui/**/*.rs` found zero `pub mod`, no audit needed); `__bindings_arms!` cross-crate test (subagent finding #10 — `#[doc(hidden)]` is supported-surface-out, not worth dedicated test); `set_focused` consistency (subagent finding #4 — already consistent); Phase 10 builder-level cross-crate test (subagent finding #15 — Phase 10 already lists end-to-end builder tests).

### Phase 6 — Pane identity, ctx, Framework skeleton ✅

The chicken-and-egg unit. `AppContext::framework()` returns `&Framework<Self>` and `Framework<Ctx>` requires `Ctx: AppContext`, so they must land together. `AppContext::set_focus` takes `FocusedPane<Self::AppPaneId>`, so the pane-id types come along.

Add:

- `tui_pane/src/pane_id.rs` — `FrameworkPaneId::{Keymap, Settings, Toasts}`, `FocusedPane<AppPaneId>::{App, Framework}`.
- `tui_pane/src/app_context.rs` — `AppContext` trait (`type AppPaneId: Copy + Eq + Hash + 'static`, `framework`, `framework_mut`, `set_focus`). The `AppPaneId` super-trait set mirrors the `Action` trait (Phase 3, renamed from `ActionEnum` in the commit preceding Phase 6) and is required by Phase 11's `HashMap<Ctx::AppPaneId, fn(&Ctx) -> Mode<Ctx>>` registry. **`set_focus` ships with a default body** that delegates to `self.framework_mut().set_focused(focus)` — binaries override only when they need extra side-effects (logging, telemetry). The two required methods are then just `framework()` / `framework_mut()`.
- `tui_pane/src/framework/mod.rs` — `Framework<Ctx>` **skeleton** (three fields, five methods, frozen):

```rust
pub struct Framework<Ctx: AppContext> {
    focused: FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,
}

impl<Ctx: AppContext> Framework<Ctx> {
    pub fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self { ... }
    pub fn focused(&self) -> &FocusedPane<Ctx::AppPaneId>           { ... }
    pub fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) { ... }
    pub fn quit_requested(&self) -> bool                            { ... }
    pub fn restart_requested(&self) -> bool                         { ... }
}
```

The `quit_requested` / `restart_requested` flags are set by the framework's internal dispatch when the user fires `GlobalAction::Quit` / `Restart` (post-Phase-3 review decision: framework owns dispatch). The binary's main loop polls these every tick and tears down accordingly. This replaces the pre-review design where the binary supplied positional `quit` / `restart` callbacks.

> **Phase 6 → Phase 11 contract.** This 3-field / 5-method API (all five methods `const fn`) is **frozen at Phase 6 and must survive Phase 11 verbatim.** Phase 11 is purely additive: it adds the `keymap_pane` / `settings_pane` / `toasts` fields, the `mode_queries` / `editor_target_path` / `focused_pane_mode` plumbing, the `dismiss()` method (framework dismiss chain), and any new query methods — but it **never renames** the five frozen methods or the three frozen fields, and **never drops `const`** from any of them. Tests written in Phases 7–10 against this surface stay green when Phase 11 lands.

No pane fields, no `mode_queries`, no `editor_target_path`, no `focused_pane_mode` in Phase 6 — those land in Phase 11 once framework panes exist.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use framework::Framework;`, `pub use pane_id::{FocusedPane, FrameworkPaneId};`, `pub use app_context::AppContext;`. Apply rule 4 (`#[must_use]`) to every getter on `Framework<Ctx>`.

#### Retrospective

**What worked:**
- The 3-file split (`pane_id.rs`, `app_context.rs`, `framework/mod.rs`) compiled together cleanly — the chicken-and-egg between `Framework<Ctx>` and `AppContext` resolved without a single forward-declaration tweak.
- `set_focus` default body (added during planning Q&A) means cargo-port's eventual `impl AppContext for App` will need only `framework()` + `framework_mut()`. Implementor surface is now 2 required methods, not 3.
- Cross-crate test in `tests/macro_use.rs` exercises the full `Framework::new` / `set_focus` / `focused()` chain through a fresh `CrossCrateApp` — locked by standing rule 6.

**What deviated from the plan:**
- **All five `Framework<Ctx>` methods are `const fn`.** Plan signatures showed plain `fn`; clippy `missing_const_for_fn` (nursery) flagged every one (including `set_focused(&mut self)` since Rust 1.83 const-mut). Result: `Framework::new`, `focused`, `set_focused`, `quit_requested`, `restart_requested` are all const. The 3-field / 5-method frozen contract is unchanged in name and signature, but the const qualifier is now part of the surface.
- **`framework/` directory holds only `mod.rs`.** Per the plan, Phase 11 fills in *panes* under `panes/`, not under `framework/`. So `framework/` will have only `mod.rs` for the foreseeable future. Kept the directory layout per plan rather than collapsing to `framework.rs`.

**Surprises:**
- **`clippy::use_self` (nursery) fires inside struct definitions, not just impl blocks.** Test-side `struct TestApp { framework: Framework<TestApp> }` was flagged; fix is `Framework<Self>`. Same applied to `CrossCrateApp` in `tests/macro_use.rs`. Worth noting for any future test fixture that holds back-references.
- **Single-variant test enums emit `dead_code` on unused variants.** `CrossCratePaneId` initially had `Alpha + Beta`; `Beta` was for "demonstrate the enum has multiple variants" but never constructed. Compiler flagged it. Reduced to `Alpha` only — the Phase 6 cross-crate test does not need to demonstrate variant distinction (that's `pane_id.rs`'s internal job).

**Implications for remaining phases:**
- **Standing rule 4 (`#[must_use]`) and clippy nursery's `missing_const_for_fn` overlap.** Every getter that's `&self` returning a value needs *both* `#[must_use]` and `const`. Apply pre-emptively in Phase 7 (`Shortcuts::visibility`/`state`/`vim_extras`/`dispatcher`), Phase 9 (`Keymap` getters), Phase 11 (new `Framework` getters added by panes).
- **`AppContext::set_focus` already exists with a default body.** Phase 7's `Shortcuts::dispatcher` and Phase 10's builder hooks call into `Framework::set_focused` via the context, not directly. No new surface needed in those phases just for focus changes.
- **Phase 11's "purely additive" rule must include `const fn`.** Adding non-const methods to `Framework<Ctx>` is fine, but any *modification* of an existing const fn signature (e.g. dropping `const`) is a regression of the Phase 6 surface.
- **`framework/` will grow with Phase 11.** Even though the plan says Phase 11 adds files to `panes/`, the additive Phase 11 work *inside* `Framework<Ctx>` will likely justify `framework/dispatch.rs` or similar private siblings. Standing rule 1 still applies — `mod` (private) declarations only.

#### Phase 6 Review

- **Standing rule 9 added** (`#[must_use]` + `const fn` on every `&self` value-returning method; `const fn` on `&mut self` setters where eligible). Codifies the Phase 6 retrospective lesson into a numbered standing rule.
- **Phase 6 → Phase 11 contract** (both the original block and its Phase 11 mirror) amended to read "5 frozen methods, all `const fn`" — `const` is now part of the frozen surface, not just an implementation detail.
- **Phase 7 prep block** adds a doc-only note that cross-crate test fixtures use multi-variant enums to avoid `dead_code` on derived impls; defaults `state` / `vim_extras` are flagged as const-eligible while `label`'s const-ness is deferred to clippy.
- **Phase 10 plan body** gains the framework-dispatcher landing: `tui_pane/src/framework/dispatch.rs` (private sibling) wires `GlobalAction` to `Framework`'s `pub(super) const fn request_quit` / `request_restart` setters, focus changes, and the optional `on_quit` / `on_restart` / `dismiss_fallback` hooks.
- **Phase 10 builder hooks** firing-order pinned: `on_quit` / `on_restart` fire **after** the framework flag is set; hook bodies can rely on `ctx.framework().quit_requested() == true`.
- **Phase 10 `dismiss_fallback` test** weakened to "hook is reachable and stored"; full chain integration moves to Phase 11 once the framework dismiss chain exists.
- **Phase 11 prelude** acknowledges that mixing `const fn` (Phase 6 methods) and plain `fn` (Phase 11 additions like `dismiss`, `editor_target_path`, `focused_pane_mode`) inside the same `impl Framework<Ctx>` is intentional. Adds explicit "`Toasts<Ctx>` is held inline, not boxed" ownership note.
- **Phase 11 `Framework` struct rewrite** restructured into "Phase 6 frozen fields (unchanged)" and "Phase 11 additions" sections so a literal-reading implementer cannot accidentally drop the frozen fields.
- **Phase 11 `focused_pane_mode` callsite** documented: `&App` is passed where `&Ctx` is expected; `Ctx == App` for cargo-port.
- **Phase 13** adds an `impl AppContext for App` line item with a note that `set_focus` defaults out — only `framework()` / `framework_mut()` are required.
- **Phase 18 regression suite** adds a "set_focus is the single funnel" test: an override impl that counts calls observes every framework focus change.

**Reviewed and not changed:**
- Finding #6 (macro-emitted const fn): user feedback — const is opportunistic, clippy gates it; do not escalate const-eligibility as a finding requiring approval (saved as `feedback_const_opportunistic.md`).

### Phase 7 — Scope traits ✅

> **Note on shipped vs. described surface.** Phase 7's actual code commit (`8f657cc`) shipped a **pre-redesign** form of the scope traits: `Shortcuts<Ctx>: 'static` (no `Pane` supertrait), `type Variant`, `fn label(&self, …) -> Option<&'static str>`, `fn input_mode() -> fn(&Ctx) -> InputMode`, and the `InputMode { Static, Navigable, TextInput }` enum. The deliverables list below describes the **post-redesign** surface adopted by the doc-sweep commit (`5cacb7b`) — `Pane<Ctx>` supertrait, `type Actions`, `Mode<Ctx>` with handler-in-variant, `Visibility` axis. **Phase 8 brings code into alignment with this description.** Until Phase 8 lands, the on-disk code lags the doc by exactly that delta — intentional, recorded here so a reader who diffs Phase 7 deliverables against `tui_pane/src/keymap/{shortcuts,navigation,globals}.rs` is not surprised.

**Cross-crate test fixtures must use multi-variant enums.** Phase 7 adds `Pane<Ctx>` / `Shortcuts<Ctx>` / `Navigation<Ctx>` / `Globals<Ctx>` smoke tests in `tests/macro_use.rs` (standing rule 6). Per the Phase 6 retrospective surprise: single-variant test enums emit `dead_code` because the lint ignores derived impls. Use multi-variant fixtures (e.g. `CrossCrateNavAction::{Up, Down, Left, Right}`); if a single-variant fixture is unavoidable, gate the unused variant with `#[allow(dead_code, reason = "...")]`.

Files (one per trait — each is independent, the heaviest is `Shortcuts<Ctx>` with 6 methods + 1 const + 1 assoc type):

- `tui_pane/src/pane.rs` — `Pane<Ctx>` with `const APP_PANE_ID: Ctx::AppPaneId` and `fn mode() -> fn(&Ctx) -> Mode<Ctx>` (default `|_| Mode::Navigable`). The supertrait for every per-pane trait. The framework registry stores the returned `mode` pointer keyed by `AppPaneId`; pane-internal callers write `Self::mode()(ctx)`.
- `tui_pane/src/keymap/shortcuts.rs` — `Shortcuts<Ctx>: Pane<Ctx>` with `type Actions: Action;` and method set per `core-api.md` §4: `defaults`, `visibility`, `state`, `bar_slots`, `vim_extras`, `dispatcher`, plus `SCOPE_NAME` const. Default `visibility` returns `Visibility::Visible`; default `state` returns `ShortcutState::Enabled`; default `bar_slots` emits `(PaneAction, Single(action))` per `Self::Actions::ALL` in declaration order. Per-pane impls override only when one of these axes is state-dependent. The bar **label** is always `Action::bar_label()` from `action_enum!` — there is no per-frame label override on the trait. `vim_extras() -> &'static [(Self::Actions, KeyBind)]` defaults to `&[]` (cargo-port's `ProjectListAction` overrides for `'l'`/`'h'` in Phase 13).
- `tui_pane/src/keymap/navigation.rs` — `Navigation<Ctx>` with `type Actions: Action;`.
- `tui_pane/src/keymap/globals.rs` — `Globals<Ctx>` with `type Actions: Action;` (app-extension globals, separate from the framework's own `GlobalAction` from Phase 3). The trait has **no** `bar_label(action) -> &'static str` method — Phase 5's `Action::bar_label` (live on every action enum, including the macro-generated and the hand-rolled `GlobalAction`) is the single source. Bar code calls `action.bar_label()` regardless of scope.

`keymap/action_enum.rs` holds the `Action` trait and the `action_enum!` macro.

**`Shortcut` wrapping struct is dead.** Phase 5 split it into orthogonal `Visibility` and `ShortcutState` axes. Phase 7 prep verifies `core-api.md` and `paneid-ctx.md` reference no `Shortcut\b` (singular wrapping struct) — `Shortcuts`, `ShortcutState`, `Visibility` are the only valid forms.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use pane::{Pane, Mode};` and `pub use keymap::{Shortcuts, Navigation, Globals};`. `keymap/mod.rs` adds `pub use shortcuts::Shortcuts; pub use navigation::Navigation; pub use globals::Globals;`. Inner files declare `mod shortcuts; mod navigation; mod globals;` (private — standing rule 1).

**Implications for later phases (locked here):**
- **Phase 9 `Keymap<Ctx>` container** relies on `Shortcuts::SCOPE_NAME`, `Navigation::SCOPE_NAME` (defaults to `"navigation"`), `Globals::SCOPE_NAME` (defaults to `"global"`) for TOML table dispatch. The default-impl test in `globals.rs` confirms `Globals<TestApp>::SCOPE_NAME == "global"` so the `[global]` table can carry both framework `GlobalAction` and the app's `Globals` impl simultaneously. Build entry point is `KeymapBuilder::build_into(&mut Framework<Ctx>) -> Result<Keymap<Ctx>, KeymapError>`. Binary constructs `Framework::new(initial_focus)` first, then hands it to the builder. The registry write is a single locus.
- **Phase 10 builder** populates the framework's per-pane registries by walking `Pane::mode()` for each registered `P: Pane<Ctx>` and storing the returned `fn(&Ctx) -> Mode<Ctx>` keyed by `P::APP_PANE_ID`. Because `mode` is a free fn returning a bare `fn` pointer, the builder needs only `P` as a type parameter — never a typed `&P` instance. Standing rule 9's `const fn` clause applies to inherent methods only — trait-default bodies can't be `const fn` in stable Rust. `const fn` with `&mut self` requires Rust ≥ 1.83 (verified before the `pub(super) const fn request_quit/request_restart` setters land).
- **Phase 11 `Framework<Ctx>::focused_pane_mode`** dispatches through the registry without holding a typed `&PaneStruct`. The default `|_ctx| Mode::Navigable` is what panes that don't override fall back to. `Framework<Ctx>::mode_queries` is private; the only writer is `pub(super) fn register_app_pane(&mut self, id: Ctx::AppPaneId, query: fn(&Ctx) -> Mode<Ctx>)`. Framework panes do not impl `Shortcuts<Ctx>` (the trait requires `APP_PANE_ID: Ctx::AppPaneId`, which framework panes lack); each framework pane (`KeymapPane`, `SettingsPane`, `Toasts`) ships inherent `defaults() / handle_key() / mode() / bar_slots()` methods directly on the struct; bar renderer + dispatcher special-case `FocusedPane::Framework(_)`. Framework pane input handling is inherent `pub fn handle_key(&mut self, ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome` — `KeyOutcome::{Consumed, Unhandled}`.
- **Phase 12 (bar render)** writes region-suppression rules in terms of `framework.focused_pane_mode(ctx)` rather than `P::mode()(ctx)` — the renderer holds a `FocusedPane`, not a typed `P`. The bar renderer calls `Keymap::render_app_pane_bar_slots(id, ctx)` and the input dispatcher calls `Keymap::dispatch_app_pane(id, &bind, ctx)`; both are `AppPaneId`-keyed and consume `RenderedSlot` / `KeyOutcome` directly. The crate-private `RuntimeScope<Ctx>` trait (renamed from `ErasedScope`) carries the per-pane vtable but is invisible to external callers — they go through the three concrete public methods on `Keymap<Ctx>`.
- **Phase 13 (mode override)** closure body: `fn mode() -> fn(&App) -> Mode<App> { |ctx| ... }`, reading state by navigating from `ctx`, not via `&self`. The Finder is the first concrete `TextInput(_)` user — its handler is migrated from binary-side `handle_finder_key` into a free fn referenced from the `Mode::TextInput(...)` variant. The `action_enum!` 3-positional form was locked in Phase 5 and is exercised by `tests/macro_use.rs`; Phase 13's migration is per-call-site only. `ProjectListAction::ExpandRow`/`CollapseRow` vim-extras override goes through `Shortcuts::vim_extras() -> &'static [(Self::Actions, KeyBind)]`.
- **Phase 18 (vim test)** adds a row-rendering check: `VimMode::Enabled` → `ProjectListAction::ExpandRow`'s bar shows `→/l`, `CollapseRow`'s shows `←/h`.

### Phase 8 — Trait redesign: `Pane<Ctx>` split, `Mode<Ctx>`, `Visibility` ✅

Phase 7 shipped (`8f657cc`) the original `Shortcuts<Ctx>: 'static` surface — `type Variant`, `fn label(&self, …) -> Option<&'static str>`, `fn input_mode() -> fn(&Ctx) -> InputMode`, and the `InputMode { Static, Navigable, TextInput }` enum. The doc-sweep at `5cacb7b` rewrote the trait API to a redesigned form (split `Pane<Ctx>` supertrait, `Mode<Ctx>` with handler-in-variant, `Visibility` axis, `type Actions` rename) but did not touch code. **Phase 8 is the code-only commit that brings the shipped traits into alignment with the redesigned doc surface.** No new container types, no new framework features — strictly an API redesign at the `Shortcuts` / `Navigation` / `Globals` surface plus the new `Pane<Ctx>` supertrait.

**Why this is its own phase.** Per the no-sub-commit rule, the redesign is a separable concern from the Phase 9 `Keymap<Ctx>` container. Bundling them would put two unrelated commits' worth of changes in one diff; splitting them keeps each commit's blast radius tight. Phase 9 (Keymap container) builds on the post-redesign surface — `P::mode()`, `Mode<Ctx>`, `type Actions` — so Phase 8 must land first.

**New file: `tui_pane/src/pane.rs`.** Holds both the supertrait and the `Mode<Ctx>` enum.

```rust
pub trait Pane<Ctx: AppContext>: 'static {
    const APP_PANE_ID: Ctx::AppPaneId;
    fn mode() -> fn(&Ctx) -> Mode<Ctx> { |_| Mode::Navigable }
}

pub enum Mode<Ctx: AppContext> {
    Static,
    Navigable,
    TextInput(fn(KeyBind, &mut Ctx)),
}
```

- `APP_PANE_ID` moves here from `Shortcuts`. Pane identity is a `Pane`-trait concern, separate from shortcut configuration.
- `mode()` returns a fn pointer (not a closure) so the framework can store it in `Framework<Ctx>::mode_queries` (Phase 9) keyed by `AppPaneId` without lifetime grief.
- `Mode<Ctx>::TextInput(handler)` bundles the handler in the variant. **This makes "TextInput pane without handler" unrepresentable** — the type system enforces that any pane in `TextInput` mode has a defined per-key handler. Replaces the prior `InputMode::TextInput` (no handler) which left handler routing as a separate concern with no compile-time link.
- `Mode<Ctx>` does **not** derive `PartialEq` (fn pointers don't `Eq`-compare cleanly). Tests use `matches!(mode, Mode::Navigable)` rather than `==`. The `Static` and `Navigable` variants are payload-free, so `matches!` is enough.

**New enum:** `pub enum Visibility { Visible, Hidden }`. Lives in `tui_pane/src/bar/visibility.rs` (sibling of `region.rs` / `slot.rs`). Bevy variant names; no `Inherited`. Slots without an override default to `Visible`. Re-exported via `pub use bar::Visibility;` at the crate root.

**Modified: `tui_pane/src/keymap/shortcuts.rs`.**
- Supertrait change: `pub trait Shortcuts<Ctx: AppContext>: Pane<Ctx>` (was `: 'static`). The `'static` bound is inherited transitively through `Pane<Ctx>: 'static`.
- Drop `const APP_PANE_ID` (moved to `Pane`).
- Drop `fn input_mode() -> fn(&Ctx) -> InputMode` (replaced by `Pane::mode -> fn(&Ctx) -> Mode<Ctx>`).
- Drop `fn label(&self, action, _ctx) -> Option<&'static str>`. The bar label is always `action.bar_label()` (static, declared in `action_enum!`). Per-frame "show vs. hide" decisions move to `visibility`.
- Add `fn visibility(&self, _action: Self::Actions, _ctx: &Ctx) -> Visibility { Visibility::Visible }`. Override when a pane drops a slot from the bar based on state — e.g. `ProjectListAction::Activate` returns `Visibility::Hidden` when no row is selected.
- Rename `type Variant: Action` → `type Actions: Action`. Update every `Self::Variant` → `Self::Actions` inside the trait body, the default `bar_slots` impl, the `vim_extras` signature, and the `dispatcher` signature.

**Modified: `tui_pane/src/keymap/navigation.rs` and `globals.rs`.** Rename `type Variant` → `type Actions`; no other changes.

**Modified: `tui_pane/src/bar/`.** Delete the `InputMode` module/enum entirely (its replacement, `Mode<Ctx>`, lives in `pane.rs` because it carries the `Ctx` parameter — `bar/` cannot host generic-over-`Ctx` types since `BarSlot<A>` and `BarRegion` are `Ctx`-free). Add `bar/visibility.rs` with the `Visibility` enum. Update `bar/mod.rs`: drop `mod input_mode;` and `pub use input_mode::InputMode;`, add `mod visibility; pub use visibility::Visibility;`.

**Modified: `tui_pane/src/lib.rs`.**
- Add `mod pane;` declaration.
- Drop `pub use bar::InputMode;`.
- Add `pub use pane::{Pane, Mode};`.
- Add `pub use bar::Visibility;`.

**Modified: existing `cfg(test)` modules.**
- `tui_pane/src/keymap/shortcuts.rs::tests`:
  - Add `impl Pane<TestApp> for FooPane { const APP_PANE_ID: TestPaneId = TestPaneId::Foo; }` (no `mode()` override — default `Navigable`).
  - Drop `const APP_PANE_ID` from the `impl Shortcuts` block.
  - Rename `type Variant` → `type Actions`.
  - Drop `default_label_returns_action_bar_label` test (method removed).
  - Replace `default_input_mode_returns_navigable` with `default_mode_returns_navigable`: build `let query = <FooPane as Pane<TestApp>>::mode();` and assert `matches!(query(&app), Mode::Navigable)`.
  - Add `default_visibility_returns_visible`: `assert_eq!(pane.visibility(FooAction::Activate, &app), Visibility::Visible);`.
- `tui_pane/src/keymap/navigation.rs::tests` and `globals.rs::tests`: rename `type Variant` → `type Actions`.

**Modified: `tui_pane/tests/macro_use.rs`.**
- `use tui_pane::{Pane, Mode, Visibility}` added; `use tui_pane::InputMode` removed.
- `impl Pane<CrossCrateApp> for CrossCratePane { const APP_PANE_ID: CrossCratePaneId = CrossCratePaneId::Alpha; }`.
- `impl Shortcuts<CrossCrateApp> for CrossCratePane` drops `const APP_PANE_ID`, renames `type Variant` → `type Actions`.
- Bar primitives smoke test: replace `InputMode::Navigable / Static / TextInput` with `Mode::Navigable / Static / TextInput(no_op_handler)` where `fn no_op_handler(_: KeyBind, _: &mut CrossCrateApp) {}`. Use `matches!` for assertions instead of `assert_eq!` (no `PartialEq` on `Mode<Ctx>`). Add a `Visibility::Visible` / `Visibility::Hidden` round-trip equality test.
- `Navigation` and `Globals` impls rename `type Variant` → `type Actions`.

**Tests added (per-trait test module):**
- `default_mode_returns_navigable` — `<P as Pane<Ctx>>::mode()(&ctx)` matches `Mode::Navigable`.
- `default_visibility_returns_visible` — `pane.visibility(action, &ctx) == Visibility::Visible`.
- `Mode::TextInput` constructor smoke test — build with a no-op fn pointer; assert `matches!(_, Mode::TextInput(_))`.
- `Visibility` round-trip — `Visible == Visible`, `Visible != Hidden`.

**Tests removed:**
- `default_label_returns_action_bar_label` — method removed from the trait.

**Out of scope (lands in Phase 9 — Keymap container):**
- `Framework<Ctx>::mode_queries` field, `register_app_pane` writer, and `focused_pane_mode(&self, &Ctx)` reader. Those land alongside `Keymap<Ctx>` in Phase 9 because the registry is the consumer of the fn pointer that `Pane::mode()` returns; without a container that registers panes there is no caller. Until Phase 9, `<P as Pane<Ctx>>::mode()` is reachable through the trait method only.

**Phase 7 → Phase 8 contract.** The Phase 7 trait surface is **deliberately broken** in this phase. There are no binary call sites yet (the binary swap is Phase 13), so the only consumers are the tui_pane crate's own `cfg(test)` modules and `tests/macro_use.rs` — both rewritten in this phase. Tests written against the Phase 7 surface (e.g. `FooPane::input_mode()`, `pane.label(...)`) are explicitly replaced, not preserved.

**Root re-exports (per Phase 5+ standing rule 2):** crate root gains `pub use pane::{Pane, Mode};` and `pub use bar::Visibility;`; loses `pub use bar::InputMode;`. The `core-api.md` §11 re-exports list is the source of truth for the post-Phase-8 surface.

**Standing-rule check.** New `Visibility` enum gets `#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]` (matches `ShortcutState`). New `Mode<Ctx>` enum gets `#[derive(Clone, Copy, Debug)]` only — `Eq`/`Hash`/`PartialEq` cannot be derived because of the fn pointer. `Pane<Ctx>::APP_PANE_ID` is a const, no `#[must_use]` needed. `Pane<Ctx>::mode()` returns a fn pointer, no `#[must_use]` (fn pointers without side effects don't trigger the lint, but flag at code review time).

**Definition of done.**
- `cargo build` clean from a fresh checkout.
- `cargo nextest run` green for the workspace.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- `rg -F 'InputMode' tui_pane/` returns zero matches.
- `rg -F 'type Variant' tui_pane/` returns zero matches.
- `rg -F 'Shortcuts::label\|fn label(' tui_pane/src/keymap/` returns zero matches.
- The `tests/macro_use.rs` smoke tests still pass against the new surface.

### Retrospective

**What worked:**
- Trait split landed as designed: `Pane<Ctx>` supertrait carries `APP_PANE_ID` + `mode()`, `Shortcuts<Ctx>: Pane<Ctx>` carries the rest. No call-site churn outside `cfg(test)` modules and `tests/macro_use.rs` since no binary code consumes the traits yet.
- `Mode<Ctx>::TextInput(fn(KeyBind, &mut Ctx))` compiled cleanly. Tests use `matches!` (not `==`) — `Mode<Ctx>` deliberately does not derive `PartialEq`/`Eq`/`Hash` because the fn-pointer payload doesn't compare cleanly.
- The doc grep checks (`InputMode`, `type Variant`, `fn label`) all return zero matches in `tui_pane/`.

**What deviated from the plan:**
- The `must_use` predictions in the plan's "Standing-rule check" were wrong: clippy pedantic flagged both `Pane::mode()` and `Shortcuts::vim_extras()` as `must_use_candidate`. Added `#[must_use]` to both. Clippy also flagged `clippy::missing_const_for_fn` on `tests/macro_use.rs::no_op_text_input` — fixed with `const fn`.
- A stale `InputMode` reference in `tui_pane/src/app_context.rs` doc comment (line 27, on `AppPaneId`) wasn't called out by the plan but had to be updated to `Mode<Ctx>`. Greppable doc comments past the trait surface need a sweep at trait-redesign time.
- Doc-markdown lint flagged un-backticked `TextInput` in two pane.rs doc comments — fixed at the same time as `must_use`.

**Surprises:**
- `Pane<Ctx>::mode()` had to be added to `app_context.rs`'s registry comment (registry doesn't exist until Phase 9, but the comment forward-references it). Reasonable to leave the comment pointing forward; just kept it consistent with the new type name.
- The `register::<P>()` calling convention is unblocked for Phase 9: `P::APP_PANE_ID` and `P::mode()` are both reachable through the `Pane<Ctx>` trait alone, so the registry writer in Phase 9 needs only `P: Pane<Ctx>`, not `P: Shortcuts<Ctx>`.

**Implications for remaining phases:**
- Phase 9 builder/registry can take `P: Pane<Ctx>` (not `Shortcuts<Ctx>`) for `mode_queries` registration, decoupling input-mode wiring from shortcut configuration. This was implicit in the redesign but worth naming so the Phase 9 prompt doesn't accidentally over-constrain.
- Phase 13 trait-tutorial walkthroughs (`### Pane<Ctx>` etc.) need a 4-column table per `feedback_trait_method_table.md`. The current trait surface is small enough that one table per trait is the right granularity.
- The "out of scope (lands in Phase 9)" callout in this phase named `Framework<Ctx>::mode_queries`, `register_app_pane`, and `focused_pane_mode`. **Correction (post-review):** framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) do not go through `register_app_pane` — they lack `AppPaneId`, are constructed inline by `Framework::new()`, and the `mode_queries` registry is for app panes only. `pub(super)` visibility on `register_app_pane` is correct (Phase 9 / 10 / 11 all already say `pub(super)`); the registry writer is internal to the keymap/framework module pair. No `pub(crate)` widening needed.

### Phase 8 Review

- **Phase 9** (Keymap container): renamed stale `Globals::Variant` / `G::Variant` to `Globals::Actions` / `G::Actions` in the dual-source `[global]` merge note (finding 1).
- **Phase 9** (Keymap container): added an explicit "registry constraint" paragraph stating `Framework<Ctx>::register_app_pane` takes `P: Pane<Ctx>` (not `Shortcuts<Ctx>`) so non-shortcut consumers can register (finding 2).
- **Phase 9** (Keymap container): added a verify-step on the `KeyParseError → KeymapError` `#[from]` chain — confirm the variant exists in the shipped Phase 4 enum or add it as a Phase 9 deliverable (finding 5).
- **Phase 11** (Framework panes): rewrote the Toasts paragraph to point at the unified `defaults() / handle_key() / mode() / bar_slots()` inherent surface instead of the inconsistent `dispatch(action, ctx)` mention (finding 7).
- **Phase 11** (Framework panes): noted that `editor_target_path`, `focused_pane_mode`, and `dismiss` are Phase-11 additions not yet documented in `core-api.md` §10, and named the §10 doc-sync sweep as part of this phase (finding 8).
- **Phase 13** (App action enums + impls): added a "no per-impl `#[must_use]`" callout on the `mode()` override snippet — the trait declaration carries it, override bodies inherit (finding 10).
- **Phase 18** (Regression tests): clarified that snapshot tests parameterize on `FocusedPane` and drive through `focused_pane_mode` + `Keymap::render_app_pane_bar_slots`, not a typed `P::mode()` call (finding 12).
- **Phase 8 retrospective** (correction): retracted the "Phase 9 should publish `register_app_pane` as `pub(crate)`" implication — framework panes don't go through registration (they lack `AppPaneId`), so `pub(super)` is correct (finding 3, approved).
- **`core-api.md` §6** (doc-of-record sweep): renamed `P::Action` / `N::Action` / `G::Action` → `P::Actions` / `N::Actions` / `G::Actions` in `scope_for` / `navigation` / `globals` lookups (finding 6, approved & applied).
- **Phase 9** (Keymap container, ErasedScope redesign): replaced the unworkable `action_for(&KeyBind) -> Option<&dyn Action>` / `display_keys_for(&dyn Action) -> &[KeyBind]` surface with three operation-level methods — `dispatch_key(&KeyBind, &mut Ctx) -> KeyOutcome`, `render_bar_slots(&Ctx) -> Vec<RenderedSlot>`, `key_for_toml_key(&str) -> Option<KeyBind>`. Typed access is captured inside the impl block (`ConcreteScope<Ctx, P>`) at registration time; the trait surface stays type-parameter-free. Phase 9 also gains a `RenderedSlot` struct (region/label/key/state/visibility) and re-uses `KeyOutcome` from Phase 11 (finding 4, approved & applied).

### Phase 9 — Keymap container ✅

> **Post-reset note (read first):** The Phase 9 review's Find 2 (`scope_for_typed`) and Find 17 (deferred collapse via `PendingEntry`) were reverted by the **Phase 9 reset** below. The Phase 9 surface that ships in the codebase today is: `pub(crate) RuntimeScope<Ctx>` (renamed from `ErasedScope`), `pub(super) PaneScope<Ctx, P>` (renamed from `ConcreteScope`), three concrete public methods on `Keymap` (`dispatch_app_pane` / `render_app_pane_bar_slots` / `key_for_toml_key`), `AppPaneId`-keyed storage only, typestate `KeymapBuilder<Ctx, State>`. The text below describes the original Phase 9 design as it shipped *before* the reset; jump to the Phase 9 reset subsection for the current state.

Add `Keymap<Ctx>` in `tui_pane/src/keymap/mod.rs` (the keymap module's anchor type lives in its `mod.rs` file, mirroring the Phase 6 precedent of `Framework<Ctx>` in `framework/mod.rs`). `Keymap<Ctx>` exposes `scope_for` / `scope_for_app_pane` / `navigation` / `globals` / `framework_globals` / `config_path` (per §6). Fill in the actual TOML-parsing implementation in `keymap/load.rs` (skeleton + `KeymapError` from Phase 4). Construction is via the canonical entry point `Keymap::<Ctx>::builder()` — an inherent associated function on `Keymap<Ctx>` that returns `KeymapBuilder<Ctx>` — no positionals (the framework owns `GlobalAction` dispatch; see Phase 3 review for full rationale). The builder body itself lands in Phase 10.

**Three scope lookups, one for each consumer.**
- `scope_for::<P>() -> Option<&dyn ErasedScope<Ctx>>` is `TypeId<P>`-keyed and erased; used by code that already has the type parameter and wants to dispatch / render / TOML-lookup through the trait surface.
- `scope_for_typed::<P>() -> Option<&ScopeMap<P::Actions>>` is `TypeId<P>`-keyed and **typed**; used by Phase 14/16 callers that want to test whether a key resolves to a specific action without firing the dispatcher (e.g. `scope_for_typed::<FinderPane>().and_then(|s| s.action_for(&bind)) == Some(FinderAction::Confirm)`). Implementation: `ErasedScope` carries `as_any(&self) -> &dyn Any`; `scope_for_typed` downcasts the trait object to `ConcreteScope<Ctx, P>` and returns `&self.bindings`. **Lands as a Phase 9 amendment at the start of Phase 10.**
- `scope_for_app_pane(id: Ctx::AppPaneId) -> Option<&dyn ErasedScope<Ctx>>` is `AppPaneId`-keyed and used by the bar renderer (Phase 12) and the input dispatcher, both of which hold a `FocusedPane` and never a typed `P`. The `AppPaneId` index is populated at `register::<P>()` time on `P::APP_PANE_ID`. (Framework panes are not in this map; they are special-cased by `FocusedPane::Framework` arms in callers — see Phase 11.)

**`ErasedScope<Ctx>` design.** Lives in `tui_pane/src/keymap/erased_scope.rs`. Shipped as `pub trait ErasedScope: sealed::Sealed + 'static` (sealed — external crates can name it but cannot implement it; only the in-crate `ConcreteScope<Ctx, P>` does). The earlier draft visibility (`pub(crate)`) made every method dead code in the non-test build because the only constructor lives in the builder and the only callers live in test modules — sealing keeps the "no external impls" intent without the dead-code tax. Each method is a complete pane operation — typed access happens **inside** the impl, where `P: Shortcuts<Ctx>` is in scope; the trait surface itself is type-parameter-free. The earlier draft (returning `&dyn Action`) is unworkable because (a) `Action` is not object-safe (`const ALL: &'static [Self]` and `: Copy + 'static`), and (b) the dispatcher signature `fn(P::Actions, &mut Ctx)` cannot be called from a `&dyn Action` — the framework has no `<P>` parameter at dispatch time, so it cannot bridge erased → typed. The fix is to bake the typed dispatch / render / lookup steps into the impl block at registration time and expose only erased-uniform return values.

```rust
mod sealed { pub trait Sealed {} }

pub trait ErasedScope<Ctx: AppContext>: sealed::Sealed + 'static {
    /// Resolve a keybind to an action and call the pane's dispatcher.
    /// `Consumed` = matched and fired; `Unhandled` = no binding for this key.
    fn dispatch_key(&self, bind: &KeyBind, ctx: &mut Ctx) -> KeyOutcome;

    /// Bar slots already reduced to label + key + state + visibility.
    /// Slots with `Visibility::Hidden` OR no bound key are dropped from the returned Vec.
    fn render_bar_slots(&self, ctx: &Ctx) -> Vec<RenderedSlot>;

    /// Reverse lookup: TOML key string → bound `KeyBind` (for keymap overlay).
    fn key_for_toml_key(&self, key: &str) -> Option<KeyBind>;
}

pub struct RenderedSlot {
    pub region:     BarRegion,
    pub label:      &'static str,
    pub key:        KeyBind,
    pub state:      ShortcutState,
    pub visibility: Visibility,
}

pub(crate) struct ConcreteScope<Ctx: AppContext, P: Shortcuts<Ctx>> {
    pane:     P,
    bindings: ScopeMap<P::Actions>,
}

impl<Ctx: AppContext + 'static, P: Shortcuts<Ctx>> sealed::Sealed for ConcreteScope<Ctx, P> {}

impl<Ctx: AppContext + 'static, P: Shortcuts<Ctx>> ErasedScope<Ctx> for ConcreteScope<Ctx, P> {
    fn dispatch_key(&self, bind: &KeyBind, ctx: &mut Ctx) -> KeyOutcome {
        self.bindings.action_for(bind).map_or(KeyOutcome::Unhandled, |action| {
            P::dispatcher()(action, ctx);
            KeyOutcome::Consumed
        })
    }

    fn render_bar_slots(&self, ctx: &Ctx) -> Vec<RenderedSlot> {
        self.pane.bar_slots(ctx)
            .into_iter()
            .filter_map(|(region, slot)| {
                let action = slot.primary();
                let visibility = self.pane.visibility(action, ctx);
                if matches!(visibility, Visibility::Hidden) { return None; }
                let key = self.bindings.key_for(action).copied()?;  // drop unbound
                Some(RenderedSlot {
                    region,
                    label: action.bar_label(),
                    key,
                    state: self.pane.state(action, ctx),
                    visibility,
                })
            })
            .collect()
    }

    fn key_for_toml_key(&self, key: &str) -> Option<KeyBind> {
        let action = P::Actions::from_toml_key(key)?;
        self.bindings.key_for(action).copied()
    }
}
```

**`KeyOutcome` lands in Phase 9.** Two variants: `Consumed` (matched and dispatched), `Unhandled` (no binding for this key — caller continues to globals / dismiss / fallback). Phase 11 re-uses the same enum on framework-pane inherent `handle_key` methods, so the dispatch loop reads one return type across app panes (via `ErasedScope::dispatch_key`) and framework panes (via inherent `handle_key`).

**`BarSlot::primary()`** is a small inherent method on `BarSlot<A>` (defined in Phase 5) returning the first action in the slot — `Single(a)` returns `a`; `Paired(a, _, _)` returns `a`. The bar renderer uses the primary action for label/key/state lookup; the second action in `Paired` is rendered alongside as the "alternate" indicator without a separate state lookup.

**Registry constraint: `P: Pane<Ctx>`, not `P: Shortcuts<Ctx>`.** The `mode_queries` registry writer (`Framework<Ctx>::register_app_pane`) needs `P::APP_PANE_ID` and `P::mode()` only, both reachable through the `Pane<Ctx>` supertrait alone. Phase 9 should declare the writer as `pub(crate) fn register_app_pane<P: Pane<Ctx>>(...)` so non-shortcut consumers (text-input routing, future bookkeeping) can register without dragging in a `Shortcuts<Ctx>` impl. The `scope_for::<P>()` and `scope_for_typed::<P>()` lookups naturally require `P: Shortcuts<Ctx>` because they walk the keymap; only `register_app_pane` is the relaxed-constraint form.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use keymap::{Keymap, KeymapBuilder, KeyOutcome, RenderedSlot, ErasedScope};`. The `Keymap::new`-style internal constructor that the builder calls is `pub(super)` (standing rule 3 — framework-only construction). The `scope_for / scope_for_app_pane` getters ship as `pub` (not `pub(crate)`) because returning `&dyn ErasedScope<Ctx>` from a `pub fn` requires the trait to be at least `pub`; the trait stays sealed so external impls remain blocked. Apply `#[must_use]` (standing rule 4) to every getter on `Keymap<Ctx>`.

**Loader-layer decisions established here (Zed/VSCode/Helix-aligned):**
- **`bar_label` is code-side only.** The third positional literal in every `action_enum!` arm is a compile-time string; the TOML loader never reads or writes it, and there is no `[bar_labels]` (or analogous) table.
- **Letter-case normalization.** `KeyBind::parse` (Phase 2) preserves case verbatim — `"Ctrl+K"` parses to `KeyCode::Char('K')`, not `Char('k')`. The TOML loader normalizes:
  - **Single-letter keys are lowercased.** `"Q"` and `"q"` both bind `Char('q')`. `"Shift+q"` is the only way to bind Shift+q (canonical), and `"Shift+Q"` normalizes to the same. Bare `"Q"` is treated as user typo for `"q"`, not as `Shift+q`.
  - **Multi-char tokens are not normalized.** `"Tab"`, `"F1"`, `"PageUp"` parse via Phase 2's case-sensitive `parse_keycode` — lowercase variants like `"tab"` are rejected. (If we later want case-insensitive named tokens, that's a Phase 2 parser change, not a loader change.)
  - **Modifier names are case-insensitive on input.** `"ctrl+q"`, `"Ctrl+q"`, `"CTRL+q"` all parse identically. The loader pre-normalizes modifier tokens before handing the string to `KeyBind::parse`. Writeback emits canonical capitalized form (`Ctrl+q`).
  - Loader uses string-based parsing via `KeyBind::parse` — **no** serde derive on `KeyBind` itself.
- **Modifier display order.** `KeyBind::display` emits `Ctrl+Alt+Shift+key` (frozen by Phase 2 tests). User TOML may write modifiers in any order (commutative), but help-overlay / bar render uses the canonical order. The loader does not preserve original ordering on round-trip; if config-export ever lands, that phase owns the round-trip-fidelity decision.

Unit tests (additions for this phase):
- `quit = "Q"` in TOML binds `Char('q')` (loader lowercasing), not `Char('Q')` and not `Shift+q`.
- `quit = "Shift+Q"` and `quit = "Shift+q"` both bind `Char('q')` with `SHIFT` (lowercase the letter, keep modifier).
- `quit = "Shift+Ctrl+k"` binds `Char('k')` with `CONTROL | SHIFT` (commutative parse).
- `quit = "ctrl+q"` and `quit = "CTRL+q"` parse identically to `"Ctrl+q"` (loader lowercases modifier tokens before parse, then `KeyBind::parse` accepts canonical).
- `quit = "tab"` is **rejected** with `KeymapError` (multi-char tokens are case-sensitive — Phase 2 parser contract).
- `KeyParseError` from `KeyBind::parse` chains into `KeymapError` via `#[from]` — round-trip a malformed binding string and assert the source error is preserved (`err.source().is_some()`). **Verify the `KeymapError::KeyParse(#[from] KeyParseError)` variant exists in the shipped Phase 4 enum before relying on it; if missing, add it as part of Phase 9 rather than treating it as a unit-test concern.**
- Unknown action in TOML (e.g. `[project_list] activte = "a"`) surfaces `KeymapError::UnknownAction { scope: "project_list", action: "activte" }` — the loader calls `A::from_toml_key(key)` and lifts `None` into the error variant with the scope name attached. Trait method stays as `Option<Self>` (no scope context); error context lives at the loader.
- **Dual-source `[global]` table merge.** Both `tui_pane::GlobalAction` (framework) and the app's `Globals::Actions` (binary) declare `SCOPE_NAME = "global"` so they share one TOML table. For each entry under `[global]`, the loader tries `GlobalAction::from_toml_key(key)` first; on `None`, falls back to `G::Actions::from_toml_key(key)`; on a second `None`, surfaces `KeymapError::UnknownAction { scope: "global", action: <key> }`. Unit test: `[global] frobnicate = "x"` errors; `[global] find = "f"` (app) and `[global] quit = "Q"` (framework) both succeed in the same file.

Vim-mode handling moved to Phase 10 (see "Vim mode — framework capability" §): vim binds are applied **inside `KeymapBuilder::build()`**, not the loader. Phase 9's loader is vim-agnostic.

### Retrospective

**What worked:**
- Sealed-trait pattern (`pub trait ErasedScope: sealed::Sealed + 'static`, single `ConcreteScope` implementor) cleared every dead-code warning without introducing a single `#[allow]` — the trait is `pub` (so consumers can name it) but external impls are blocked at the type system.
- Filtering unbound bar slots inside `render_bar_slots` (rather than calling `KeyBind::default()`) avoided introducing a meaningless `Default` impl on `KeyBind`. Hidden + unbound now share one drop path.
- Splitting `Keymap::insert_scope_raw` / `insert_pane_id_raw` from the builder kept the typed-vs-erased registration boundary symmetric — builder owns the typed `<P>` parameter, keymap stores erased values.

**What deviated from the plan:**
- `ErasedScope<Ctx>` shipped as `pub trait ... : sealed::Sealed + 'static` instead of `pub(crate) trait`. Reason: keeping it `pub(crate)` made every method dead code in the non-test build (the only constructors live in the builder, the only callers live in test modules). Sealing the trait preserved the "external crates cannot extend" intent the `pub(crate)` was meant to enforce.
- `Keymap::scope_for` / `scope_for_app_pane` are `pub` (not `pub(crate)`) for the same reason: returning `&dyn ErasedScope<Ctx>` from a `pub fn` requires the trait to be at least `pub`.
- `RenderedSlot::key` is `KeyBind`, not `KeyBind::default()` on miss. Slots without a binding are filtered out of the returned `Vec` (joining the `Visibility::Hidden` filter) — `KeyBind` has no `Default` impl and the plan's `unwrap_or_default()` snippet would not have compiled.
- Added `Ctx: 'static` bounds at four impl/struct sites (`Keymap`, `KeymapBuilder`, `ConcreteScope::new`, `ErasedScope` impl). `Box<dyn ErasedScope<Ctx>>` defaults to `'static` lifetime; without the bound the storage refused to compile. The plan implied this was free from `Pane<Ctx>: 'static` but `Ctx` itself was unbounded.
- `KeymapError::KeyParse(#[from] KeyParseError)` variant: did not exist in the Phase 4 enum, added it now. The Phase 4 enum already had `InvalidBinding { source: KeyParseError }` for scoped errors; `KeyParse` is the unscoped `?`-propagation form the plan called out as a verify-step.
- `BarSlot::primary()` shipped as a `const fn` (not just an inherent method) — opportunistic per the const-eligibility memory.

**Surprises:**
- The "ErasedScope is internal scaffolding" framing in the plan implied `pub(crate)` was the correct visibility. In practice, returning a trait object from a `pub fn` makes any visibility narrower than the trait itself unworkable — every consumer (Phase 11 dispatcher, Phase 12 bar renderer) must name the trait. Sealing is the actual privacy lever, not visibility.
- The plan's `key: ...unwrap_or_default()` line silently assumed `Default for KeyBind`. The cleaner fix (filter unbound slots) collapses two render-time skip paths into one and removes a meaningless default value.
- `Box<dyn Trait>` storage requires `'static` on every type parameter that appears in the trait, not just the trait's own super-bound. `AppContext` itself does not require `'static`, so the bound has to be added at the storage site.

**Implications for remaining phases:**
- Phase 10 builder body inherits the `Ctx: AppContext + 'static` bound — wire it consistently across `with_settings`, `on_quit`, `on_restart`, `dismiss_fallback`, `vim_mode`. The skeleton already has it.
- Phase 11 input dispatcher reaches `dispatch_key` through `Keymap::scope_for_app_pane(id)?.dispatch_key(...)`. The `KeyOutcome::Unhandled` variant is the chain-continue signal. Framework panes use the same enum on inherent methods.
- Phase 12 bar renderer iterates `Keymap::scope_for_app_pane(id)?.render_bar_slots(ctx)` and consumes `RenderedSlot` directly — the typed `Action` enum never crosses the trait, so the renderer is generic over no `<A>` parameter.
- Phase 10 should NOT introduce a `register_navigation` / `register_globals` that returns trait objects — those scopes are singletons (one impl per app), so direct typed storage by `TypeId<N>` / `TypeId<G>` matches the existing core-api §6 design and avoids paying the erasure tax twice.
- ~~core-api.md §6 / §7 need to sync to reflect the shipped surface: `pub trait ErasedScope` (sealed), `pub fn scope_for/scope_for_app_pane`, `Ctx: AppContext + 'static`. Plus the §7 `KeymapBuilder` body still lists positional args (`on_quit`, `on_restart`, `dismiss_fallback`, `vim`) that the Phase 9 skeleton does not have yet — those land in Phase 10. Apply the doc-sync pass during the Phase 10 review, not eagerly.~~ **Shipped with the Phase 9 reset.** core-api.md §6 / §7 now describe the post-reset surface (typestate builder, `pub(crate) RuntimeScope`, three concrete public methods on `Keymap`).

### Phase 9 reset (post-review simplification)

After Phase 9 review's amendments landed, the user pushed back on accumulated complexity at the keymap boundary. A `/ask_a_friend` consultation with Codex confirmed the diagnosis: erasure itself is justified (runtime callers hold `AppPaneId`, not `P`), but several pieces around it did not earn their keep. The reset removes them and replaces the flat builder with a typestate.

**Cuts (what shipped, then went away):**
- `as_any` method on the erased trait + `scope_for_typed::<P>` typed accessor + `bindings()` accessor on the wrapper struct — these existed only for tests/inspection. Tests now go through `dispatch_key` and observe side effects.
- `by_type: HashMap<TypeId, ...>` primary index + `by_pane_id: HashMap<AppPaneId, TypeId>` secondary index. Runtime callers hold `FocusedPane`, which carries `AppPaneId` not `TypeId<P>`. Code that names `P` already has `P`'s methods directly. The TypeId index had no callers.
- `scope_for::<P>()` and `scope_for_typed::<P>()` public lookups — same root cause. Dropped both.
- `PendingEntry` trait + `PendingScope` struct (Find 17 amendment). Replaced with eager collapse inside `register::<P>` once the typestate enforces "settings before panes."
- `scope_for_app_pane(id) -> Option<&dyn ErasedScope<Ctx>>` public getter — replaced with three concrete public methods on `Keymap<Ctx>`: `dispatch_app_pane(id, bind, ctx)`, `render_app_pane_bar_slots(id, ctx)`, `key_for_toml_key(id, action)`. The renamed `RuntimeScope<Ctx>` trait is `pub(crate)`; external callers never name it.
- Sealed-trait pattern (`sealed::Sealed` marker module). Unnecessary once the trait is `pub(crate)`.

**Renames:**
- `ErasedScope<Ctx>` → `RuntimeScope<Ctx>` (file `erased_scope.rs` → `runtime_scope.rs`).
- `ConcreteScope<Ctx, P>` → `PaneScope<Ctx, P>`. Visibility narrowed from `pub(crate)` to `pub(super)`. No more `new` constructor — fields are `pub(super)` and the builder constructs directly with a struct literal.

**Adds:**
- Typestate on `KeymapBuilder<Ctx, State>`. `State` defaults to `Configuring`; the first `register::<P>` call returns `KeymapBuilder<Ctx, Registering>`. `Configuring` exposes settings methods (`config_path` now; Phase 10's `load_toml`, `vim_mode`, `with_settings`, `register_navigation`, `register_globals`, `on_quit`, `on_restart`, `dismiss_fallback`); `Registering` exposes only `register` and `build` / `build_into`. Compile-fail doctest on `KeymapBuilder` verifies the ordering rule.
- `Keymap::dispatch_app_pane`, `Keymap::render_app_pane_bar_slots`, `Keymap::key_for_toml_key` — the three concrete methods replacing `scope_for_app_pane`. Each returns a sensible value (`KeyOutcome::Unhandled` / empty `Vec` / `None`) when the `AppPaneId` is not registered.

**Tests after the reset:** 112 pass (109 before the reset + 3 new tests at the `Keymap` boundary: `render_app_pane_bar_slots_resolves_through_keymap`, `render_app_pane_bar_slots_empty_for_unregistered_pane`, `register_chains_in_registering_state`) plus one `compile_fail` doctest on `KeymapBuilder` for the typestate rule.

**Why this matters for Phase 10:** the Phase 9-amendment work that built `PendingEntry` + `PendingScope` to defer `into_scope_map()` collapse to `build()` is no longer needed — `register::<P>` does the typed work inline (defaults → TOML overlay → vim extras → collapse) because the typestate guarantees `load_toml` and `vim_mode` are already in the builder when `register` runs. Phase 10 wires those settings methods onto the `Configuring` state and consumes them inline in `register`.

#### Phase 9 reset Review

Architect review of remaining phases against the post-reset surface produced 8 findings. 7 applied to the plan text. 1 (Phase 17 cleanup list) was a confirmation pass — no edit needed.

- **Phase 10 doc-sync prerequisite already shipped** (Find 1, applied): obsolete bullet in Phase 9 Review block now strikethrough'd, marked "shipped with the Phase 9 reset."
- **Phase 16 Esc-on-output uses reverse-lookup, not a typed probe** (Find 2, applied): rewrote the snippet to call `keymap.key_for_toml_key(OutputPane::APP_PANE_ID, OutputAction::Cancel.toml_key())` and compare against `bind`. No new public method, no `<P>`-typed probe re-introduced — Phase 16 reuses the existing public reverse-lookup.
- **`with_navigation` / `with_globals` → `register_navigation` / `register_globals`** (Find 3, applied): startup example at line 381 updated, module-tree comment at line 57 updated.
- **Phase 17 cleanup list** (Find 4, no edit): confirmed the list names no reset-removed types. `PaneScope::new` aside is the only spot mentioning a renamed type, and that's already accurate.
- **Phase 13 overlay walks the binary's known pane set** (Find 5, applied): documented that the binary supplies `(P::APP_PANE_ID, P::Actions::ALL)` pairs to the overlay; no `Keymap::registered_app_panes()` getter required.
- **Phase 10 → Phase 11 sequencing** (Find 6, applied): added a one-line "Hard dependency on Phase 10" note at the top of Phase 11. Phase 10 already includes the typed singleton getters; the note just makes the dependency explicit so future readers don't think Phase 11 is independently buildable.
- **`framework_globals` registration path** (Find 7, applied): added a Phase-10-body paragraph saying `framework_globals` is constructed inline at `build()` from `GlobalAction`'s defaults plus the shared `[global]` TOML table. The builder does not expose a `register_framework_globals` method.
- **`build()` is for tests only post-Phase-10** (Find 8, applied): added a Phase-10-body note that production code uses `build_into(&mut framework)` exclusively; `build()` exists only for unit tests that don't need a `Framework<Ctx>`. Type-system enforcement would require a third typestate; rustdoc + reviewer awareness is the lever.

#### Retrospective

**What worked:**
- `/ask_a_friend` consultation with Codex confirmed the user's "this feels overengineered" diagnosis fast — the conversation cost a few minutes and produced concrete cuts.
- Typestate `Configuring` / `Registering` pattern landed cleanly in 50 lines of builder code; the `compile_fail` doctest verifies the ordering rule with no `trybuild` dependency.
- Eager collapse in `register::<P>` removed both `PendingEntry` and `PendingScope` without losing any test coverage — typed work happens with `P` in scope, no boxing of typed accumulators required.
- Three concrete public methods on `Keymap` (`dispatch_app_pane` / `render_app_pane_bar_slots` / `key_for_toml_key`) replace the `pub fn scope_for_app_pane(id) -> Option<&dyn ErasedScope<Ctx>>` getter; the trait went `pub(crate) RuntimeScope`.

**What deviated from the plan:**
- The plan called for a `compile_fail` test as "optional"; user wanted it added, so it shipped on `KeymapBuilder`'s rustdoc.
- The plan didn't list `register_app_pane::<P>` as something to drop, but the no-op stub had no Phase-9-era backing storage so it went away. Phase 11 reintroduces it when `mode_queries` lands on `Framework<Ctx>`.
- `core-api.md` §6/§7 sync was the "first task of Phase 10" per the original plan; the reset folded it forward into the reset commit, so Phase 10 starts without a doc-sync prerequisite.
- A test was added at the `Keymap` boundary (`render_app_pane_bar_slots_resolves_through_keymap`) the original plan didn't call out — it was a gap once `scope_for_app_pane` went away.

**Surprises:**
- `unnecessary_wraps` clippy lint fired on `finalize` — Phase 9's plan had `build()` return `Result` for forward compatibility with Phase 10 errors, but the helper fn that does the work doesn't need to wrap. Solution: helper returns `Keymap<Ctx>`, `build` wraps in `Ok(...)`. Phase 10 will tighten this when real errors land.
- The Phase 9 retrospective and Phase 9 Review blocks both documented the to-be-reverted amendments (`scope_for_typed`, `PendingEntry`) — those entries now read as "shipped → reverted." Annotated with `~~strikethrough~~` rather than deleted, because the reasoning behind the original choice is still useful context.

**Implications for remaining phases:**
- **Phase 10:** every settings method (`load_toml` / `vim_mode` / `with_settings` / `register_navigation` / `register_globals` / `on_quit` / `on_restart` / `dismiss_fallback`) lives on the `Configuring`-state impl. `register::<P>` is the one method that exists on both states, with identical bodies — Phase 10's typed work (TOML overlay, vim extras) runs inside that body, not in a deferred `build()` walk.
- **Phase 11:** dispatcher calls `keymap.dispatch_app_pane(id, &bind, ctx)`, not the (now-gone) `scope_for_app_pane(id)?.dispatch_key(...)` chain. `KeyOutcome::Unhandled` semantics unchanged.
- **Phase 12:** bar renderer calls `keymap.render_app_pane_bar_slots(id, ctx)` for the `App(id)` arm. Framework-pane arm unchanged.
- **Phase 14/16:** can no longer use `scope_for_typed::<P>().and_then(...)` to probe an action without dispatching. Two options: (a) dispatch through `dispatch_app_pane` and observe a side effect (atomic counter, captured value), (b) add a `cfg(test) pub(crate)` typed-action probe at the phase that needs it. The plan now points to (a) by default; (b) lands per-phase if the test really needs it.
- **Phase 18:** keymap overlay reads typed singletons (`Keymap::navigation::<N>` / `Keymap::globals::<G>`) — those are typed by design, unaffected by the reset.

### Phase 9 Review

Architect review of remaining phases against shipped Phase 9 produced 18 findings. 11 minor were applied silently to the plan text. 7 significant were reviewed with the user; outcomes below.

**Phase 9 amendments to land at the start of Phase 10** (since Phase 9 already shipped):
- ~~**Add typed scope accessor** (Find 2): `Keymap::scope_for_typed::<P>()`.~~ **Reverted by Phase 9 reset.** Test/inspection access doesn't belong on the public erased trait. Tests go through `dispatch_app_pane` and observe side effects.
- ~~**Defer `into_scope_map()` collapse to `build()`** (Find 17).~~ **Reverted by Phase 9 reset.** Eager collapse in `register::<P>` works once the typestate enforces "settings before panes." Phase 10's TOML overlay and vim extras land inline in `register::<P>` instead of via deferred collapse.

**Phase 10 plan changes:**
- **One error type, not two** (Find 12): `KeymapError` gains three variants (`NavigationMissing`, `GlobalsMissing`, `DuplicateScope { type_name: &'static str }`) and remains the sole failure type. `BuilderError` is dropped from the plan and from core-api §7. `KeymapBuilder::build()`'s signature stays `Result<Keymap<Ctx>, KeymapError>` — Phase 9 tests do not change.
- **Typed singleton storage for `Navigation` / `Globals`** (Find 13): `Keymap<Ctx>` adds `navigation: Box<dyn Any + Send + Sync>` and `globals: Box<dyn Any + Send + Sync>` fields, populated by `KeymapBuilder::register_navigation::<N>()` and `register_globals::<G>()`. Public getters: `pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Actions>` and `pub fn globals<G: Globals<Ctx>>(&self) -> &ScopeMap<G::Actions>`. Pane scopes stay erased (heterogeneity is the reason); singletons stay typed (Phase 18's `key_for(NavigationAction::Up)` reads through the public getter, no downcast at the call site). `framework_globals: ScopeMap<GlobalAction>` already typed — unchanged.
- **TOML overlay merges into `Bindings<A>` before collapse** (Find 17 layering): `KeymapBuilder::load_toml(path)` walks each scope's TOML table, calls `A::from_toml_key(toml_key)` to resolve the action, parses the value with `KeyBind::parse`, and pushes into the pending `Bindings<P::Actions>` accumulator. `into_scope_map()` runs once per scope inside `build()` — never during `register` or during loader passes.
- **Vim extras applied inside `build()`** (existing plan note, reaffirmed): `KeymapBuilder::build()` walks each pending scope; for each `(action, key)` in `P::vim_extras()`, skip if `key` is already bound in the current `Bindings<A>` (not just same `KeyCode` — the full `KeyBind`), else append. Vim merge happens before `into_scope_map()` collapse.
- **`Ctx: 'static` bound on every builder hook** (Find 3 minor): `with_settings`, `on_quit`, `on_restart`, `dismiss_fallback`, `vim_mode` all live in `impl<Ctx: AppContext + 'static>` — no per-method addition needed. Plan text confirms this rather than restating it per-method.

**Phase 11 plan changes — dispatch chain matches existing cargo-port behavior (Finds 1, 6, 10):**

```text
Pre-flight (binary-specific structural escapes — match existing cargo-port):
  1. Esc + framework example_running (or app's equivalent) → kill PID, return.
  2. Esc + non-empty output buffer → clear, refocus, return.
  3. Confirm modal active → consume key (y/n only), return.

Then match focused pane:

  FocusedPane::Framework(fw_id):
    Overlay panes intercept ALL keys when focused. The overlay's
    inherent handle_key(...) returns KeyOutcome::Consumed regardless
    (overlays never delegate). Open-overlay state is the cargo-port
    rule today — keep it.

  FocusedPane::App(id):
    a. Framework globals first: keymap.framework_globals().action_for(bind)
       → if Some, framework dispatches (Quit/Restart/NextPane/PrevPane/
       OpenKeymap/OpenSettings/Dismiss). Returns Consumed on hit.
    b. App globals next: keymap.globals::<G>().action_for(bind) → if Some,
       G::dispatcher() runs. Returns Consumed on hit. (The shared
       [global] TOML table merges both sources at load time — see Phase 9
       loader-decisions.)
    c. Navigation scope: keymap.navigation::<N>().action_for(bind) → if
       Some, N::dispatcher() routes by FocusedPane to the focused
       scrollable surface. Returns Consumed on hit. (Existing cargo-port
       hardcodes nav per-pane; the trait centralizes routing.)
    d. Per-pane scope: keymap.dispatch_app_pane(id, bind, ctx).
       Returns Consumed or Unhandled (Unhandled if no scope is
       registered for `id` or no binding matches).
    e. Unhandled → drop the key (no further fallback).

Dismiss is the named global action, not an Unhandled fallback:
  GlobalAction::Dismiss → framework dismiss chain
    → toasts.try_pop_top() (only if toasts pane is the dismiss target)
    → focused-overlay close
    → dismiss_fallback hook (binary's optional opt-in)

Toasts::handle_key fires only when FocusedPane::Framework(Toasts) is
focused — it lives in the App(id) → b/c/d chain only by virtue of
focus being on the toasts pane (matching cargo-port's PaneBehavior::Toasts
arm). Visible-but-not-focused toasts ignore key input.
```

**Phase 11/14/16 snippet rewrites (Find 1, post-reset):** every plan snippet of the form `keymap.scope_for::<P>().action_for(&bind) == Some(SomeAction)` is replaced by either (a) dispatching through `keymap.dispatch_app_pane(P::APP_PANE_ID, &bind, ctx)` and observing the dispatcher's side effect, or (b) a `cfg(test) pub(crate)` test-only typed-action probe added in the affected phase. The Phase 9 reset dropped `scope_for_typed`; Phase 14 (Finder Confirm/Cancel) and Phase 16 (Esc-on-output) take the dispatch-and-observe form. Phase 18's keymap overlay reads typed singletons (`Keymap::navigation::<N>` / `Keymap::globals::<G>`) and the typed-public-method form is unchanged from Phase 10's plan.

**Phase 12 plan changes (Find 6):**
- The bar renderer matches `FocusedPane` first. Only the `App(id)` arm calls `Keymap::render_app_pane_bar_slots(id, ctx)` and consumes `RenderedSlot`. The `Framework(fw_id)` arm calls each framework pane's inherent `bar_slots()` method directly — the keymap is never queried for framework-pane bar contents.
- Region modules (`nav_region`, `pane_action_region`, `global_region`) filter the flat `Vec<RenderedSlot>` by `region` field — they no longer walk typed `BarSlot<A>` payloads. Replace plan wording that names tuple patterns like `(Nav, _)` with field-match on `RenderedSlot { region: BarRegion::Nav, .. }`.

**Phase 13 plan changes (Find 7, Find 8 minor):**
- `Keymap` overlay drives off `P::Actions::ALL` (from the `Action` trait), then calls `keymap.key_for_toml_key(P::APP_PANE_ID, action.toml_key())` per action to fetch the bound key. Unbound actions render with an empty key cell so the user can rebind them — `render_bar_slots` (which drops unbound) is the wrong API for the overlay; that one is the **status bar's** API. The overlay walks the registered pane set by iterating the binary's known list of `(P::APP_PANE_ID, P::Actions::ALL)` pairs — the binary already knows its panes, so no `Keymap::registered_app_panes()` getter is required.
- `key_for_toml_key` returning `None` for "unknown action" vs "known action, no binding" is treated identically by the overlay (both render as unbound). The trait method does not need to distinguish them.

**Phase 17 plan changes (Find 14 minor):** Phase 17's `const fn` deletion list applies only to the pre-refactor binary types (`Shortcut::from_keymap` / `disabled_from_keymap`). New `tui_pane` `const fn`s (`BarSlot::primary`, framework `const fn` getters) are kept — do not run a careless `s/const fn/fn/` sweep. (`PaneScope` no longer has a `new` constructor post-reset — fields are `pub(super)` and the builder constructs with a struct literal.)

**Phase 18 plan changes (Finds 9, 15 minor):**
- Dispatch parity tests assert via the dispatcher's side effect (atomic counter, captured value), not the return. `KeyOutcome::Consumed` only tells the caller a binding fired; *which* action fired is observed through the dispatcher.
- Add a `KeymapError::KeyParse` propagation regression test: round-trip a malformed binding string through the loader, assert the variant matches and the source is preserved.

**Findings rejected:** none. All seven significant findings produced plan changes; the 11 minor findings either applied directly (where actionable) or confirmed existing plan text (no change needed).

**Doc-sync:** core-api.md §6/§7 are now synced to the post-reset surface (typestate builder, `pub(crate) RuntimeScope`, three concrete public methods on `Keymap`, no `BuilderError`, single `KeymapError`, `Ctx: AppContext + 'static`). No further §6/§7 work required at the start of Phase 10.

### Phase 10 — Keymap builder + settings registry

**Phase 9 reset already shipped:** the builder skeleton has the typestate (`Configuring` → `Registering`), `register::<P>` does eager collapse, the public surface is three concrete methods on `Keymap` (`dispatch_app_pane` / `render_app_pane_bar_slots` / `key_for_toml_key`), and the trait is `pub(crate) RuntimeScope<Ctx>`. core-api.md §6/§7 are synced. Phase 10 adds the settings phase methods and the framework integration that hook onto that scaffolding.

Two tightly-coupled additions in one commit because `KeymapBuilder::with_settings` is the only consumer of `SettingsRegistry`:

- `tui_pane/src/settings.rs` — `SettingsRegistry<Ctx>` + `add_bool` / `add_enum` / `add_int` / `with_bounds` (§9).
- `tui_pane/src/keymap/builder.rs` — `KeymapBuilder<Ctx, Configuring>` body fills in. One error type — `KeymapError` — covers loader and builder validation; three new variants land here: `NavigationMissing`, `GlobalsMissing`, `DuplicateScope { type_name: &'static str }`. `BuilderError` was rejected during Phase 9 review (one type beats two when the binary's startup path renders both the same).

**Typed singleton storage for `Navigation` / `Globals`.** `Keymap<Ctx>` gains three fields populated by `KeymapBuilder::register_navigation::<N>()` and `register_globals::<G>()` (both on the `Configuring` state):

```rust
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:            HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>, // pane scopes (erased — heterogeneous)
    navigation:        Option<Box<dyn Any + Send + Sync>>,                  // Some(ScopeMap<N::Actions>) post-build
    globals:           Option<Box<dyn Any + Send + Sync>>,                  // Some(ScopeMap<G::Actions>) post-build
    framework_globals: ScopeMap<GlobalAction>,                              // already typed (no <Ctx>)
    config_path:       Option<PathBuf>,
}

pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Actions> { /* downcast */ }
pub fn globals<G: Globals<Ctx>>(&self)       -> &ScopeMap<G::Actions> { /* downcast */ }
```

Pane scopes stay erased (heterogeneity is the reason). Singletons stay typed — Phase 18's `key_for(NavigationAction::Up)` reads through the public getter without a downcast at the call site (the getter does it). Framework globals stay typed inline.

**`framework_globals` is constructed inline at `build()`** from `GlobalAction`'s defaults (the framework's own default bindings — Quit/Restart/NextPane/etc.) plus the shared `[global]` TOML table. The builder does not expose a `register_framework_globals` method — framework globals are non-overridable in the sense that the binary cannot replace the dispatcher, but the *bindings* are user-overridable through TOML's `[global]` table (which merges with `[<app-globals-scope>]` per Phase 9 loader-decisions). `build()` resolves the `[global]` overlay onto `GlobalAction`'s defaults and stores the result inline.

**TOML overlay applies inline at `register::<P>`.** `KeymapBuilder<Ctx, Configuring>::load_toml(path)` reads + parses the file into a `TomlTable` stored on the builder. Each subsequent `register::<P>(pane)` call (during `Configuring` and again after the typestate transition is irrelevant — the `register` body has the same logic in both states) walks the `[P::SCOPE_NAME]` table, calls `P::Actions::from_toml_key` to resolve the action, parses the value with `KeyBind::parse`, layers the override onto `P::defaults()`, then collapses to `ScopeMap<P::Actions>`. Cross-scope validation (every `[scopename]` table in the TOML must match a registered scope) runs in `build()` against the recorded `SCOPE_NAME` set. **No deferred storage, no `PendingEntry`** — `P` is in scope inside `register::<P>`, so the typed work happens inline.

**Vim extras apply inline at `register::<P>`.** Same point in the chain. If the builder has `vim_mode == VimMode::Enabled`, append `'k'/'j'/'h'/'l'` to `Navigation::UP/DOWN/LEFT/RIGHT` (skipping any already bound on the full `KeyBind`, not just the `KeyCode`); for each `(action, key)` in `P::vim_extras()`, skip if `key` is already bound, else append. Applied **after** TOML overlay so `[navigation]` user replacement does not disable vim.

**Builder hooks (post-Phase-3 review).** `KeymapBuilder<Ctx, Configuring>` exposes three optional chained hooks for framework lifecycle notification — framework owns the dispatch, hooks fire after:
- `.on_quit(fn(&mut Ctx))` — fires **after** `framework.quit_requested` is set to `true`. Hook body can rely on `ctx.framework().quit_requested() == true`.
- `.on_restart(fn(&mut Ctx))` — fires **after** `framework.restart_requested` is set to `true`. Hook body can rely on `ctx.framework().restart_requested() == true`.
- `.dismiss_fallback(fn(&mut Ctx) -> bool)` — fires when framework's own dismiss chain finds nothing to dismiss; returns `true` if binary handled it.

All three live on the `Configuring`-state impl block — the typestate enforces "settings before panes," so the hooks are recorded once, before the first `register::<P>` call captures them along with TOML and vim settings.

**Build entry point — `build_into(&mut Framework<Ctx>)`.** The terminal call on the chain (added on the `Registering`-state impl) is `.build_into(&mut framework) -> Result<Keymap<Ctx>, KeymapError>`. The binary constructs `Framework::new(initial_focus)` first, then hands the mutable reference to the builder. `build_into` populates the framework's per-`AppPaneId` registry (`mode_queries`) by calling `framework.register_app_pane(P::APP_PANE_ID, P::mode())` for each `P: Pane<Ctx>` registered on the builder (the builder records the `(P::APP_PANE_ID, P::mode())` pairs at `register::<P>` time so `build_into` doesn't need to walk typed scopes again). This keeps `Framework<Ctx>` and `Keymap<Ctx>` independently constructible (the framework exists before the keymap is built), and makes the registry write a single locus rather than threading a `Ctx` through `build()`. `register_app_pane` is `pub(super)` on `Framework` (standing rule 3); the `mode_queries` field is private.

**`build()` is for tests only post-Phase-10.** Phase 9's reset shipped `build()` on both states (no `Framework<Ctx>` integration was wired yet). Once Phase 10's `build_into` lands, production code uses `build_into` exclusively — `build()` produces a `Keymap<Ctx>` whose registered panes are *not* wired into `framework.mode_queries`, which would silently break `focused_pane_mode(ctx)` for the bar renderer and input dispatcher. Add a rustdoc note on `build()`: "Production code should call `build_into(&mut framework)` to populate the framework's mode-query registry. `build()` exists for unit tests that don't need a `Framework<Ctx>`." No type-system enforcement (typestate would need a third state); rustdoc + reviewer awareness is the lever.

**Framework dispatcher lands here.** `KeymapBuilder::build()` also wires the framework's built-in dispatcher for every `GlobalAction` variant. The dispatcher is a free fn `fn dispatch_global<Ctx: AppContext>(action: GlobalAction, ctx: &mut Ctx)` living in a new private sibling `tui_pane/src/framework/dispatch.rs` (declared `mod dispatch;` from `framework/mod.rs` per standing rule 1). It closes over the `.on_quit` / `.on_restart` / `.dismiss_fallback` hooks the binary registered on the builder. Per `GlobalAction` variant:

- `Quit` → calls `ctx.framework_mut().request_quit()` (new `pub(super)` setter on `Framework` — see below); then fires `on_quit` if registered.
- `Restart` → `ctx.framework_mut().request_restart()` (new `pub(super)` setter); then fires `on_restart` if registered.
- `NextPane` / `PrevPane` → consults the registered pane set, computes next/prev focus, calls `ctx.set_focus(new_focus)` (the `AppContext` default funnels into `framework_mut().set_focused(...)`).
- `OpenKeymap` / `OpenSettings` → `ctx.set_focus(FocusedPane::Framework(FrameworkPaneId::Keymap | Settings))`.
- `Dismiss` → end-to-end chain lands in Phase 11 (toasts → focused framework overlay → `dismiss_fallback`); Phase 10's dispatcher arm calls a Phase-10-supplied `Framework::dismiss()` method that returns `bool`. Until Phase 11, the Phase 10 dispatcher arm for `Dismiss` calls `dismiss_fallback` directly — Phase 11 inserts the framework chain in front.

**Phase 6 → Phase 11 contract addendum: `pub(super)` setters on `Framework`.** Phase 10 adds two write methods to `Framework<Ctx>` so the dispatcher (sibling of `framework/mod.rs`) can flip lifecycle flags without breaking encapsulation:

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    /// Flip `quit_requested` to `true`. Called by the framework's
    /// built-in `GlobalAction::Quit` dispatcher; not part of the
    /// public surface.
    pub(super) const fn request_quit(&mut self) { self.quit_requested = true; }

    /// Flip `restart_requested` to `true`. Called by the framework's
    /// built-in `GlobalAction::Restart` dispatcher; not part of the
    /// public surface.
    pub(super) const fn request_restart(&mut self) { self.restart_requested = true; }
}
```

These are `pub(super)` per standing rule 3 (framework-internal construction / mutation), which makes them invisible to the binary while accessible to `framework/dispatch.rs`. The Phase 6 → Phase 11 contract speaks to *public* surface; `pub(super)` additions do not violate the freeze.

`const fn` with `&mut self` writing a struct field has been stable since Rust 1.83 (Nov 2024). Verify the workspace MSRV in `Cargo.toml` is ≥ 1.83 before Phase 10 lands; if not, drop `const` from these two setters (the rest of the rule-9 const-where-eligible policy still applies).

Unit tests:
- TOML round-trip through the builder: single-key form, array form, in-array duplicate rejection.
- `KeymapError::NavigationMissing` / `GlobalsMissing` / `DuplicateScope` surface from `build()`.
- `.on_quit()` / `.on_restart()` are reachable and stored — a unit test fires the corresponding `GlobalAction` and asserts the registered hook ran. (`.dismiss_fallback()` end-to-end firing requires Phase 11's dismiss chain — the Phase 10 test only asserts the hook is reachable and stored; the chain integration test moves to Phase 11.)
- Vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods), not just `code`: if user binds `Shift+k` to anything, vim's `'k'` for `NavigationAction::Down` still applies (different mods). (Migrated from Phase 9 — vim application is the builder's job.)
- `MockNavigation::Up` keeps its primary as the inserted `KeyCode::Up` even with `VimMode::Enabled` applied — insertion-order primary preserved (deferred from Phase 4).

After Phase 10 the entire `tui_pane` foundation is in place: keys, action machinery, bindings, scope map, bar primitives, pane id + ctx + framework skeleton, scope traits, keymap, builder, settings registry. Framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) and the `Framework` aggregator's pane fields + helper methods land in Phase 11.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use settings::SettingsRegistry;` (`KeymapBuilder` and `KeymapError` are already re-exported from Phase 9; no `BuilderError` to add — see error-type decision above). `KeymapBuilder::build` and any internal helpers it calls into other keymap files are `pub(super)` (standing rule 3).

**Standing rule 9 applies to inherent methods only (per Phase 7 retro).** Trait-default method bodies cannot be `const fn` in stable Rust. For `KeymapBuilder<Ctx>`'s chain methods (`fn on_quit(mut self, …) -> Self`, `with_*`, etc.), apply `#[must_use]` — clippy's `must_use_candidate` catches the omission anyway, and the chain-style return-`Self` form is the canonical builder pattern. `Keymap<Ctx>`'s inherent getters get the full `#[must_use]` + `const fn where eligible` treatment.

### Phase 11 — Framework panes

Phase 11 fills in the framework panes inside the **existing** `Framework<Ctx>` skeleton from Phase 6. The struct's pane fields and helper methods land here; the type itself, `AppContext`, and `FocusedPane` already exist.

**Hard dependency on Phase 10.** The dispatcher chain below calls `keymap.framework_globals()`, `keymap.globals::<G>()`, and `keymap.navigation::<N>()` — all three are added by Phase 10 (typed singleton getters + the storage they read). Phase 11 cannot land until Phase 10 ships those.

**Mixing const and non-const inside `impl Framework<Ctx>` is intentional.** The five Phase 6 methods (`new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`) stay `const fn` verbatim. The Phase 11 additions (`dismiss`, `editor_target_path`, `focused_pane_mode`, etc.) call into `HashMap` lookups and pane state, neither of which is const-eligible — those land as plain `fn`. Standing rule 9 still applies (every `&self` value-returning method gets `#[must_use]`; const where eligible).

**`Toasts<Ctx>` is held inline, not boxed.** The new `toasts: Toasts<Ctx>` field lives directly on `Framework<Ctx>`. Dispatchers reach it via `ctx.framework_mut().toasts.try_pop_top()`. No `Rc`/`RefCell`/`Cell` wrappers — single-threaded ownership through `&mut Ctx` is the contract.

> **Phase 6 → Phase 11 contract (mirror).** Purely additive: this phase adds fields and methods, but the Phase 6 surface (3 frozen fields: `focused`, `quit_requested`, `restart_requested`; 5 frozen methods, **all `const fn`**: `new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`) is **frozen verbatim** — names, signatures, and `const` qualifier alike. Tests written in Phases 7–10 against the skeleton must continue to pass at the end of Phase 11. If Phase 11 implementation surfaces a better name or signature for any of the frozen items, that is a deliberate breaking change — surface it as a follow-up, not a silent rename.

Add to `tui_pane/src/panes/`:

- `keymap_pane.rs` — `KeymapPane<Ctx>` with internal `EditState::{Browse, Awaiting, Conflict}`. Method `editor_target(&self) -> Option<&Path>`. `mode(&self, ctx) -> Mode<Ctx>` returns `TextInput(keymap_capture_keys)` when `EditState == Awaiting`, `Static` when `Conflict`, `Navigable` when `Browse`.
- `settings_pane.rs` — `SettingsPane<Ctx>` with internal `EditState::{Browse, Editing}`; uses `SettingsRegistry<Ctx>`. Method `editor_target(&self) -> Option<&Path>`. `mode(&self, ctx) -> Mode<Ctx>` returns `TextInput(settings_edit_keys)` when `EditState == Editing`, `Navigable` otherwise.
- `toasts.rs` — `Toasts<Ctx>` stack with `ToastsAction::Dismiss` (defaults to `Esc`). Framework panes do **not** implement `Shortcuts<Ctx>` (the trait requires `APP_PANE_ID: Ctx::AppPaneId`, which framework panes lack — they carry `FrameworkPaneId` instead). Instead, `Toasts<Ctx>` exposes the same inherent surface as the other framework panes (see the "Framework panes carry inherent action machinery" paragraph below): `defaults()`, `handle_key()`, `mode()`, `bar_slots()` — directly on the struct, no trait. The bar renderer and dismiss chain special-case `FocusedPane::Framework(...)` arms. Under the post-Phase-3 design, `Toasts` participates in the framework's `dismiss()` chain: when `GlobalAction::Dismiss` fires, framework first asks `toasts.try_pop_top()`; if nothing was on the stack, framework checks the focused framework overlay; if still nothing, falls through to the binary's `dismiss_fallback`.

**Framework panes carry inherent action machinery, not a `Pane<Ctx>` / `Shortcuts<Ctx>` impl.** Each of `KeymapPane<Ctx>`, `SettingsPane<Ctx>`, `Toasts<Ctx>` ships:
- `pub fn defaults() -> Bindings<Self::Action>` — same role as `Shortcuts::defaults`, no trait.
- `pub fn handle_key(&mut self, ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome` — pane consumes the key or signals fallthrough (see Phase 14). `KeyOutcome::Consumed` halts dispatch; `KeyOutcome::Unhandled` lets the caller continue to globals/dismiss. **Overlay panes (`KeymapPane`, `SettingsPane`) intercept ALL keys when focused — the inherent handler returns `Consumed` regardless** (matches existing cargo-port `keymap_open` / `settings_open` short-circuit behavior). Toasts is the only framework pane whose `handle_key` can return `Unhandled`.
- `pub fn mode(&self, ctx: &Ctx) -> Mode<Ctx>` — `&self` form (the framework owns the struct directly, no split-borrow constraint).
- `pub fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Action>)>` — same role as `Shortcuts::bar_slots`.

These mirror the trait method set per-method, but as inherent methods so the `Pane<Ctx>::APP_PANE_ID` constraint doesn't apply. The bar renderer and dispatcher walk `FocusedPane::App(_)` through the trait surface and `FocusedPane::Framework(_)` through these inherent methods.

**Dispatch chain (matches existing cargo-port `src/tui/input.rs::handle_key_event` order).** The framework input dispatcher routes `KeyEvent` through this chain:

```text
Pre-flight (binary-specific structural escapes — keep cargo-port behavior verbatim):
  1. Esc + framework example_running (or app's equivalent) → kill PID, return.
  2. Esc + non-empty output buffer → clear output, refocus, return.
  3. Confirm modal active → consume key (y/n only), return.

Then match focused pane:

  FocusedPane::Framework(fw_id):
    fw_pane.handle_key(ctx, &bind)  // overlay takes the key
    Overlays return KeyOutcome::Consumed regardless. Toasts may return
    Unhandled (key fell through Toasts' own bindings). On Unhandled,
    fall through to step (a) below.

  FocusedPane::App(id) (or Framework(Toasts) → Unhandled fall-through):
    a. Framework globals first: keymap.framework_globals().action_for(&bind)
       → if Some, framework dispatches (Quit/Restart/NextPane/PrevPane/
       OpenKeymap/OpenSettings/Dismiss). Returns Consumed on hit.
    b. App globals next: keymap.globals::<G>().action_for(&bind) → if Some,
       G::dispatcher() runs. Returns Consumed on hit. (The shared
       [global] TOML table merges both sources at load time — Phase 9
       loader-decisions.)
    c. Navigation scope: keymap.navigation::<N>().action_for(&bind) → if
       Some, N::dispatcher() routes by FocusedPane to the focused
       scrollable surface. Returns Consumed on hit.
    d. Per-pane scope: keymap.dispatch_app_pane(id, &bind, ctx).
       Returns Consumed or Unhandled (Unhandled if no scope is
       registered for `id` or no binding matches).
    e. Unhandled → drop the key (no further fallback).

Dismiss is the named global action, not an Unhandled fallback:
  GlobalAction::Dismiss → Framework::dismiss(focused_dismiss_target())
    → toasts.try_pop_top() (when toasts is the dismiss target)
    → focused-overlay close (Keymap / Settings)
    → dismiss_fallback hook (binary's optional opt-in)
  This fires only when the bound key resolves to Dismiss — never on
  every Unhandled.

Toasts::handle_key fires only when FocusedPane::Framework(Toasts) is
focused (matching cargo-port's PaneBehavior::Toasts arm). Visible-but-
not-focused toasts ignore key input.
```

This is a strict generalization of today's `handle_key_event` order. The `keymap_open` / `settings_open` short-circuits become the `Framework(KeymapPane | SettingsPane)` overlay arm. The `handle_global_key` step becomes (a)+(b). `handle_normal_key`'s hardcoded nav becomes (c). Per-pane keymap dispatch becomes (d). The cargo-port behavior stays byte-identical under default bindings.

**`Framework<Ctx>::dismiss()`** is added in this phase (the Phase 6 skeleton did not have it):

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    /// Run the framework dismiss chain. Returns `true` if anything was
    /// dismissed at the framework level. Caller (the `GlobalAction::Dismiss`
    /// dispatcher) consults this; on `false`, calls the binary's
    /// registered `dismiss_fallback` (if any).
    pub fn dismiss(&mut self) -> bool {
        if self.toasts.try_pop_top() { return true; }
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => { self.set_focused(self.previous_focus()); true }
            FocusedPane::Framework(FrameworkPaneId::Settings) => { self.set_focused(self.previous_focus()); true }
            _ => false,
        }
    }
}
```

**Extend `tui_pane/src/framework/mod.rs`** — keep the three frozen Phase 6 fields and five frozen const-fn methods verbatim; add the new pane fields, the registry field, and the new methods. Do *not* rewrite the struct as a wholesale replacement; this is a strict superset of the Phase 6 skeleton.

Fields added in Phase 11 (the three Phase 6 fields stay verbatim, in their original positions):

```rust
pub struct Framework<Ctx: AppContext> {
    // ── Phase 6 frozen fields (unchanged) ──
    focused:           FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,

    // ── Phase 11 additions ──
    pub keymap_pane:   KeymapPane<Ctx>,
    pub settings_pane: SettingsPane<Ctx>,
    pub toasts:        Toasts<Ctx>,
    /// Per-AppPaneId queries supplied by the app at registration. Each
    /// app pane's `Pane::mode()` returns a `fn(&Ctx) -> Mode<Ctx>`
    /// registered alongside its dispatcher. Framework panes are
    /// special-cased and do not appear here. Private; populated
    /// through `pub(super) fn register_app_pane`, which
    /// `KeymapBuilder::build_into` calls per registered pane.
    mode_queries: HashMap<Ctx::AppPaneId, fn(&Ctx) -> Mode<Ctx>>,
}
```

Methods added in Phase 11 (the five Phase 6 const-fn methods stay verbatim — `new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`). `editor_target_path`, `focused_pane_mode`, and `dismiss` are Phase-11 additions not yet documented in `core-api.md` §10 — fold them into §10 as part of this phase's doc-sync sweep:

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    pub fn editor_target_path(&self) -> Option<&Path> {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.editor_target(),
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.editor_target(),
            _ => None,
        }
    }

    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Mode<Ctx> {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Toasts)   => Mode::Static,
            FocusedPane::App(app)                              => self.mode_queries.get(&app)
                .map_or(Mode::Navigable, |q| q(ctx)),
        }
    }
}
```

Callers pass the same `&App` they hold; the method takes `&Ctx` because the framework is generic, but `Ctx == App` in cargo-port and the `&App` derefs cleanly.

The registry is populated by `KeymapBuilder::build_into(&mut framework)`: for each `P: Pane<Ctx>` registered on the builder, the chain calls `P::mode()` (the trait associated function on `Pane<Ctx>`) to obtain the `fn(&Ctx) -> Mode<Ctx>` pointer and hands it to `framework.register_app_pane(P::APP_PANE_ID, query)`. `register_app_pane` is `pub(super)` so only the builder writes the registry; the field stays private.

`Framework<Ctx>` lives in `tui_pane` (skeleton from Phase 6; filled in here). The `App.framework: Framework<App>` field-add lands in **Phase 14**, when the framework panes' input paths replace the old `handle_settings_key` / `handle_keymap_key`. Before Phase 14 the filled-in framework type is exercised only by `tui_pane`'s own `cfg(test)` units and `tui_pane/tests/` integration files.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use panes::{KeymapPane, SettingsPane, Toasts};`. `panes/mod.rs` is `mod keymap_pane; mod settings_pane; mod toasts; pub use keymap_pane::KeymapPane; pub use settings_pane::SettingsPane; pub use toasts::Toasts;` (standing rule 1). New `Framework<Ctx>` getters (`editor_target_path`, `focused_pane_mode`, `dismiss`, etc.) get `#[must_use]` per standing rule 4 where applicable.

### Phase 12 — Framework bar renderer

Add `tui_pane/src/bar/` per the BarRegion model:

- `mod.rs` — `render(focused, ctx, keymap, framework) -> StatusBar`. Matches `focused: &FocusedPane<Ctx::AppPaneId>` first, fetches `Vec<RenderedSlot>` from the right source, walks `BarRegion::ALL`, dispatches to each region module, joins spans into `StatusBar`.
- `region.rs` — `BarRegion::{Nav, PaneAction, Global}` + `ALL` (added Phase 5).
- `slot.rs` — `BarSlot<A>`, `ShortcutState`, `BarSlot::primary` (added Phase 5 / 9).
- `support.rs` — `format_action_keys(&[KeyBind]) -> String`, `push_cancel_row`, shared row builders.

**Top-level dispatch matches `FocusedPane` first.** App panes flow through the keymap; framework panes never touch the keymap (no scope registered under their pane id):

```rust
let pane_slots: Vec<RenderedSlot> = match focused {
    FocusedPane::App(id) => keymap.render_app_pane_bar_slots(*id, ctx),
    FocusedPane::Framework(FrameworkPaneId::Keymap)   => framework.keymap_pane.bar_slots(ctx)   .resolve_keys(...),
    FocusedPane::Framework(FrameworkPaneId::Settings) => framework.settings_pane.bar_slots(ctx) .resolve_keys(...),
    FocusedPane::Framework(FrameworkPaneId::Toasts)   => framework.toasts.bar_slots(ctx)        .resolve_keys(...),
};
// region modules then partition pane_slots by `region` field.
```

Framework panes return `Vec<(BarRegion, BarSlot<Self::Action>)>` from their inherent `bar_slots()`; a small adapter resolves each to `RenderedSlot` using the framework pane's own bindings (the keymap's `framework_globals` and the inherent default keys), so every region module sees the same `RenderedSlot` value type regardless of source.

**Region modules walk `RenderedSlot { region, .. }`, not typed `BarSlot<A>` tuples.** With Phase 9's `RenderedSlot` carrying `region: BarRegion` as a flat field, the per-region modules filter by field-match — they no longer thread an `A` type parameter:

- `nav_region.rs` — emits framework's nav + pane-cycle rows when `matches!(framework.focused_pane_mode(ctx), Mode::Navigable)`, then `pane_slots.iter().filter(|s| s.region == BarRegion::Nav)`. Suppressed entirely when the mode is `Static` or `TextInput(_)`.
- `pane_action_region.rs` — emits `pane_slots.iter().filter(|s| s.region == BarRegion::PaneAction)`. Renders for `Navigable` and `Static`; suppressed for `TextInput(_)`.
- `global_region.rs` — emits `GlobalAction` + `AppGlobals::render_order()` (resolved through the same `RenderedSlot` adapter); suppressed when `matches!(framework.focused_pane_mode(ctx), Mode::TextInput(_))`.

Depends on Phase 11 (`Framework<Ctx>` exists; framework-pane `Shortcuts<Ctx>` impls exist) plus Phase 9's `Keymap<Ctx>` lookups.

Snapshot tests in this phase cover the framework panes only (Settings Browse / Settings Editing / Keymap Browse / Keymap Awaiting / Keymap Conflict / Toasts) plus a fixture pane exercising every `BarRegion` rule. App-pane snapshots land in Phase 13 once their `Shortcuts<App>` impls exist.

**Paired-row separator policy.** Inherited from the Phase 2 retrospective decision: the `Paired` row's `debug_assert!` covers only the parser-producible `KeyCode` set; widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep. See Phase 2 review block (line 1020) for full text.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::StatusBar;` (and any other public bar types not already exported in Phase 5). All `bar/` submodules declared `mod` (private) in `bar/mod.rs` per standing rule 1.

### Phase 13 — App action enums + `Shortcuts<App>` impls

**Parallel-path invariant for Phases 13–16.** The new dispatch path lands alongside the old one. The old path stays the source of truth for behavior; the new path is exercised by tests added in each phase. **Phase 17 is the only phase that deletes** old code.

**Flat-namespace paths (per Phase 5+ standing rule 2).** Every `tui_pane` import in this phase uses flat paths: `use tui_pane::KeyBind;`, `use tui_pane::GlobalAction;`, `use tui_pane::Shortcuts;`, `tui_pane::action_enum! { ... }`, `tui_pane::bindings! { ... }`. Never `tui_pane::keymap::Foo`.

**Binary-side `mod` rule (per Phase 5+ standing rule 1).** New module files added to `src/tui/` for the new action enums (e.g. `app_global_action.rs`, `navigation_action.rs`) are declared `mod foo;` at their parent (never `pub mod foo;`); facades re-export with `pub use foo::Type;`. `cargo mend` denies `pub mod` workspace-wide.

In the cargo-port binary crate:

- **`action_enum!` migration cost.** Every existing `action_enum!` invocation in `src/tui/` gains a third positional `bar_label` literal between the toml key and description, per Phase 5's grammar amendment. When the bar text matches the toml key, just duplicate the literal — no per-arm design decision. The hand-rolled `tui_pane::GlobalAction` already ships its own `bar_label` (Phase 5). The macro itself was already updated in Phase 5 and the cross-crate fixtures in `tui_pane/tests/macro_use.rs` already use the 3-positional form (verified Phase 7) — Phase 13's binary-side migration is purely a per-call-site update, not a grammar change.
- Define `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction`.
- **Split today's `GlobalAction`** in `src/tui/keymap.rs` into `tui_pane::GlobalAction` (the framework half: Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss) and `AppGlobalAction` (binary-owned). During Phases 13–16 the binary's existing `GlobalAction` stays in place; references to the framework's enum are path-qualified as `tui_pane::GlobalAction` to disambiguate. Phase 17 deletes the binary's old enum and `use tui_pane::GlobalAction` makes the name available unqualified. (Requires `pub use keymap::GlobalAction;` at `tui_pane/src/lib.rs` crate root — add this re-export when Phase 13 lands, mirroring the Phase 3 `Action` precedent.)
- Add `ExpandRow` / `CollapseRow` to `ProjectListAction`.
- Implement `Pane<App>` and `Shortcuts<App>` for each app pane (Package, Git, ProjectList, CiRuns, Lints, Targets, Output, Lang, Cpu, Finder). Each pane:
  - `Pane<App>` block declares `const APP_PANE_ID: AppPaneId` and overrides `mode()` when needed (FinderPane returns `Mode::TextInput(finder_keys)` while open, else `Mode::Navigable`; OutputPane returns `Mode::Static`; the rest accept the default `Mode::Navigable`). **Override body uses the free-fn signature** — `fn mode() -> fn(&App) -> Mode<App> { |ctx| ... }` — and the closure reads state by navigating from `ctx: &App` (e.g. `ctx.overlays.finder.is_open()`), never `&self`. **No per-impl `#[must_use]`**: the trait declaration carries it (Phase 8); override bodies inherit. The Finder's `finder_keys` free fn is migrated from `src/tui/finder.rs::handle_finder_key` (translated to take `KeyBind` + `&mut App`).
  - `Shortcuts<App>` block owns `defaults() -> Bindings<Action>`.
  - Owns `visibility(&self, action, ctx) -> Visibility` and `state(&self, action, ctx) -> ShortcutState` — moves cursor-position-dependent visibility logic out of `App::enter_action` into the affected impls (CiRuns Activate `Hidden` at EOL; Package/Git/Targets Activate `Disabled` when their preconditions fail). The bar **label** is always `Action::bar_label()`.
  - Registers a free dispatcher `fn(Action, &mut App)`.
  - Optionally overrides `bar_slots(ctx)` for paired layouts and data-dependent omission (ProjectList: emits `(Nav, Paired(CollapseRow, ExpandRow, "expand"))` and `(Nav, Paired(ExpandAll, CollapseAll, "all"))`; CiRuns: omits toggle row when no ci data).
  - Overrides `vim_extras` to declare pane-action vim binds (`ProjectListAction::ExpandRow → 'l'`, `CollapseRow → 'h'`).
- Implement `Navigation<App> for AppNavigation` and `Globals<App> for AppGlobalAction`.
- **`impl AppContext for App`** — required for `Framework<App>` to instantiate. Per Phase 6's narrowed surface, only `framework()` and `framework_mut()` need bodies; `set_focus` ships with a default that delegates to `self.framework_mut().set_focused(focus)`. cargo-port takes the default unless a focus-change side-effect (logging, telemetry) becomes useful — decide at impl time.
- Build the app's `Keymap` at startup. Old `App::enter_action` and old `for_status_bar` still exist; the new keymap is populated but not consumed yet.

**`anyhow` lands in the binary in this phase.** This is the first call site that benefits from context wrapping (`Keymap::<App>::builder(...).load_toml(path).build_into(&mut framework)?` → wrap with `.with_context(|| format!("loading keymap from {path:?}"))`). Add `anyhow = "1"` to the root `Cargo.toml` `[dependencies]`. The library (`tui_pane`) does not depend on `anyhow` — only typed `KeymapError` / `KeyParseError` / etc. cross the framework boundary, and the binary adds context at the boundary.

**Phase 13 tests:**
- CiRuns `pane.visibility(Activate, ctx)` returns `Visibility::Hidden` when the viewport cursor is at EOL (hides the slot).
- Package `pane.state(Activate, ctx)` returns `ShortcutState::Disabled` when on `CratesIo` field without a version (action visible but inert).
- Finder `Pane::mode()(ctx)` returns `Mode::TextInput(finder_keys)` while open, `Mode::Navigable` otherwise.
- Finder migration: typing `'k'` in the search box inserts `'k'` into the query (handler is sole authority — vim keybinds in other scopes do not fire).
- App-pane bar snapshot tests under default bindings: one snapshot per focused-pane context (Package / Git / ProjectList / CiRuns / Lints / Targets / Output / Lang / Cpu / Finder).

### Phase 14 — Reroute overlay input handlers

Convert overlay handlers to scope dispatch:

- The Finder's TextInput handler is the free fn `finder_keys(KeyBind, &mut App)` referenced from `Pane<App>::mode()`'s `Mode::TextInput(finder_keys)` return. While the Finder is focused and its mode is `TextInput`, the framework dispatch routes every keystroke to that handler — globals/nav scopes do not fire (the handler is sole authority). The handler dispatches Finder action keys (`Confirm`, `Cancel`) through `keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, ctx)` — `KeyOutcome::Consumed` means a Finder action fired and consumed the keystroke; `KeyOutcome::Unhandled` falls through to the literal `Char(c)` / `Backspace` / `Delete` text-input behavior. (Pre-Phase-9-reset drafts read action enum values via a typed accessor; that accessor was dropped — dispatch-and-observe replaces it.)
- Framework `SettingsPane::handle_key(&mut self, ctx, &bind) -> KeyOutcome` replaces today's `handle_settings_key` + `handle_settings_adjust_key` + `handle_settings_edit_key`. Browse/Editing modes route through internal mode flag. The dispatch caller checks the return: `KeyOutcome::Consumed` halts; `KeyOutcome::Unhandled` falls through to globals/dismiss.
- Framework `KeymapPane::handle_key(&mut self, ctx, &bind) -> KeyOutcome` replaces `handle_keymap_key`. Browse/Awaiting/Conflict modes route through internal mode flag. Same `KeyOutcome` return contract.

**`KeyOutcome` enum (introduced in Phase 9, broadened in Phase 14).** Public, two-variant: `Consumed` (pane handled the key; caller stops dispatch), `Unhandled` (caller continues to the globals chain / dismiss fallback). First defined in Phase 9 as the return type of `RuntimeScope::dispatch_key` (app-pane dispatch path, surfaced publicly through `Keymap::dispatch_app_pane`). Phase 11 re-uses the same enum on framework-pane inherent `handle_key` methods so the dispatch loop reads one return type across both surfaces. Boolean would compile, but standing rule "enums over `bool` for owned booleans" applies — the return is a domain decision (handled vs not handled), not a generic flag.

**Phase 14 tests:**
- Rebinding `FinderAction::Cancel` to `'q'` closes finder; `'k'` typed in finder inserts `'k'` even with vim mode on.
- Binding any action to `Up` while in Awaiting capture mode produces a "reserved for navigation" rejection (replaces today's `is_navigation_reserved` semantics via scope lookup).

### Phase 15 — Reroute base-pane navigation

`KeyCode::Up`/`Down`/`Left`/`Right`/`PageUp`/`PageDown`/`Home`/`End` in `handle_normal_key` (`input.rs:580-622`), `handle_detail_key`, `handle_lints_key`, `handle_ci_runs_key` consult `NavigationAction` after the pane scope. ProjectList's `Left`/`Right` route via `ProjectListAction::CollapseRow` / `ExpandRow` (pane-scope precedence). Delete `NAVIGATION_RESERVED` (`keymap.rs:794-799`) and `is_navigation_reserved` — replaced by scope lookup against `NavigationAction`.

**Phase 15 tests:**
- Rebinding `NavigationAction::Down` to `'j'` (vim-off) moves cursor.
- Rebinding `ProjectListAction::ExpandRow` to `Tab` (with `GlobalAction::NextPane` rebound away) expands current row.

### Phase 16 — Reroute Toasts, Output, structural Esc

Convert `handle_toast_key` (`input.rs:657-684`) to consult `ToastsAction::Dismiss`. The Esc-on-output structural pre-handler at `input.rs:112-119` runs before overlays/globals/pane handlers — so pressing Esc clears `example_output` from any pane. Preserve the cross-pane semantics but route the key check through the framework:

```rust
let bind = KeyBind::from(event);
if !app.inflight.example_output().is_empty()
   && !matches!(app.framework.focused_pane_mode(app), Mode::TextInput(_))
{
    // Cancel-on-output is a structural preflight that fires from any
    // pane, not just OutputPane. We need to know "would `bind`
    // dispatch to OutputAction::Cancel if OutputPane were focused?"
    // *without* running the dispatcher (the side effect would clear
    // `example_output` twice — once here, once if OutputPane is
    // focused).
    //
    // The post-Phase-9-reset answer: route through the existing
    // `key_for_toml_key(id, action_name)` reverse lookup and compare
    // against the inbound `KeyBind`. No new public method, no typed
    // probe re-introduced.
    if app.keymap
        .key_for_toml_key(OutputPane::APP_PANE_ID, OutputAction::Cancel.toml_key())
        == Some(bind)
    {
        let was_on_output = app.focus.is(PaneId::Output);
        app.inflight.example_output_mut().clear();
        if was_on_output { app.focus.set(PaneId::Targets); }
        return;
    }
}
```

The reverse-lookup form (`Action → KeyBind`) is the inverse of dispatch (`KeyBind → Action`) and is already part of the public surface for the keymap-overlay use case. Phase 16 reuses it for the structural Esc preflight rather than adding a new typed-probe public method — the post-Phase-9-reset commitment is "no public typed accessors keyed on `<P>`."

`focused_pane_mode()` returns the focused pane's `Mode<Ctx>`. The `!matches!(..., Mode::TextInput(_))` guard prevents the structural Esc from firing while a Settings numeric edit is active (where Esc means "discard edit", not "clear example_output").

After Phase 16: every key dispatches through the keymap. No `KeyCode::*` direct match for command keys remains.

**Phase 16 tests:**
- Rebinding `OutputAction::Cancel` to `'q'` clears example_output from any pane.
- Rebinding `ToastsAction::Dismiss` to `'d'` dismisses focused toast via `'d'`.
- With Settings in Editing mode, pressing Esc cancels the edit instead of clearing example_output (text-input gating).

### Phase 17 — Bar swap and cleanup

Add the `What dissolves` / `What survives` summary (currently in this doc) as user-facing notes inside `tui_pane/README.md` so the published library has its own change log of what the framework absorbed.

**Binary main loop change (post-Phase-3 review).** The binary's main loop in `src/tui/terminal.rs` switches from polling `app.overlays.should_quit()` to polling `app.framework.quit_requested()` and `app.framework.restart_requested()`. The `should_quit()` accessor on `overlays` deletes; the framework owns the lifecycle flags now. If the binary needs cleanup, it registers `.on_quit(|app| { app.persist_state() })` on the builder.

Delete:

- `App::enter_action`, `shortcuts::enter()` const fn.
- The old combined `GlobalAction` enum in `src/tui/keymap.rs` (split into `tui_pane::GlobalAction` + `AppGlobalAction` in Phase 13).
- `Overlays::should_quit` accessor and the `should_quit` flag on `Overlays` — replaced by `framework.quit_requested()`.
- The seven static constants (`NAV`, `ARROWS_EXPAND`, `ARROWS_TOGGLE`, `TAB_PANE`, `ESC_CANCEL`, `ESC_CLOSE`, `EXPAND_COLLAPSE_ALL`) and all their call sites.
- `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups` per-context helpers.
- Threaded `enter_action` / `is_rust` / `clear_lint_action` / `terminal_command_configured` / `selected_project_is_deleted` parameters.
- The dead `enter_action` arm in `project_list_groups`.
- The CiRuns `Some("fetch")` label at EOL (the bar bug).
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`. **Note:** the deletion list applies only to the pre-refactor binary types. New `tui_pane` `const fn`s (`BarSlot::primary`, framework `const fn` getters) are kept — do not run a careless `s/const fn/fn/` sweep.

After Phase 17, `shortcuts.rs` contains only legacy types pending removal (or is deleted entirely if all callers have flipped to `Shortcuts::visibility` / `Shortcuts::state`). The `InputContext` enum is deleted; tests under `src/tui/app/tests/` referencing it migrate to `app.focus.current()`-based lookups in this phase.

Hoist `make_app` from `tests/mod.rs` to `src/tui/tui_test_support.rs` (`pub(super) fn make_app`); declare `#[cfg(test)] mod tui_test_support;` in `src/tui/mod.rs`.

**Relocate framework-only tests from the binary to `tui_pane`.** Walk every `#[test]` and `#[cfg(test)] mod tests` in `src/tui/keymap.rs`, `src/tui/keymap_state.rs`, and any Phase 13-onwards test under `src/tui/` that exercises only `tui_pane` types through cargo-port's `App`. Concretely: keymap TOML loading, scope dispatch through `Keymap::scope_for`, vim-mode application by the builder, default-binding round-trips, action `from_toml_key`/`bar_label` lookups. Move each to `tui_pane/tests/` (one file per concern, e.g. `tests/keymap_loader.rs`, `tests/scope_dispatch.rs`, `tests/vim_application.rs`) against a **minimal mock context** — a small `MockApp` struct matching the one in `tui_pane/src/keymap/shortcuts.rs::tests` (a `Framework<MockApp>` field plus a tiny `MockPaneId` enum). Tests that genuinely depend on `App` state (focus transitions, toast manager, watcher integration) stay in the binary. Outcome: the framework's behavior tests live with the framework, the binary tests only what is binary-specific.

### Phase 18 — Regression tests

Bar-on-rebind:

- Rebinding each `*Action::Activate` (`Package`, `Git`, `Targets`, `CiRuns`, `Lints`) updates that pane's bar.
- Rebinding `NavigationAction::Up` / `Down` / `Left` / `Right` updates the `↑/↓` nav row in every base-pane bar that uses it.
- Rebinding `GlobalAction::NextPane` updates the pane-cycle row.
- Rebinding `ProjectListAction::ExpandAll` / `CollapseAll` updates the `+/-` row.
- Rebinding `ProjectListAction::ExpandRow` / `CollapseRow` updates the `←/→ expand` row.
- With `VimMode::Enabled`, `ProjectListAction::ExpandRow`'s bar row shows `→/l` (vim extra merged into the scope by the builder, surfaced through `display_keys_for`); `CollapseRow` shows `←/h`.
- Rebinding `FinderAction::Activate` / `Cancel` / `PrevMatch` / `NextMatch` updates the finder bar.
- Rebinding `OutputAction::Cancel` updates the output bar.
- Rebinding settings/keymap actions (framework-internal) updates their bars.

Globals + precedence:

- Globals render order matches the framework's render order then `AppGlobals::render_order()`; each slot's bar text comes from `action.bar_label()` (Phase 5's `Action::bar_label`), not a `Globals` trait method.
- CiRuns Activate at EOL renders no Enter row.
- `key_for(NavigationAction::Up) == KeyBind::from(KeyCode::Up)` even when vim mode is on.
- Rebinding `GlobalAction::Quit` to `q` keeps `q` quitting from any pane (global beats unbound).
- Rebinding `GlobalAction::NextPane` to `j` (vim-off) cycles panes from any base pane.
- Rebinding `ProjectListAction::ExpandRow` makes the pane-scope binding fire instead of `NavigationAction::Right`.
- Rebinding `FinderAction::Activate` to `Tab` while finder is open fires Activate, NOT `GlobalAction::NextPane`.
- **`AppContext::set_focus` is the single funnel.** A test impl that overrides `set_focus` to count calls observes every framework focus change (NextPane/PrevPane, OpenKeymap, OpenSettings, return-from-overlay) — locks the Phase 6 narrowed-implementor-surface contract.

Dispatch parity (per pane, the highest-risk path):

- For each `*Action::Activate` (Package/Git/Targets/CiRuns/Lints): rebind to `'a'`, synthesize an `'a'` key event, assert the pane's free-function dispatcher ran. **Assertion observed via the dispatcher's side effect** (atomic counter, captured `Cell<Option<Action>>`, etc.) — `KeyOutcome::Consumed` only signals "a binding fired"; *which* action ran is observed through the dispatcher itself.
- Rebind `AppGlobalAction::OpenEditor` to `'`'`, synthesize `'`'`, assert `open_editor` dispatched.
- Rebind `GlobalAction::Dismiss` to `Ctrl+D`, synthesize `Ctrl+D`, assert `dismiss` injected closure ran.

Vim/text-input regression:

- vim-mode on, finder open: `'k'` appends to query; cursor does not move.
- vim-mode off, finder open: `'k'` appends to query.
- finder open with `FinderAction::PrevMatch` rebound to `'k'`: `'k'` moves cursor up (FinderAction beats text input fall-through within finder).
- Settings in Editing mode: Esc cancels edit; `example_output` not cleared (text-input gating works).

TOML loader:

- `[finder] activate = "Enter"` and `cancel = "Enter"` → `Err(KeymapError::CrossActionCollision)`.
- TOML scope replaces vim+defaults: `[navigation] up = ["PageUp"]` with vim-on → `key_for(Up) == PageUp`, `'k'` not bound.
- `KeymapError::KeyParse` propagation: round-trip a malformed binding string through the loader (e.g. an unscoped `?`-propagation path that hands a bad string to `KeyBind::parse`); assert the variant matches `KeymapError::KeyParse(_)` and `err.source().is_some()` so the underlying `KeyParseError` is preserved.

A snapshot test per focused-pane context locks in byte-identical bar output to the pre-refactor bar under default bindings. The fixture drives the renderer through `framework.focused_pane_mode(ctx)` and the `AppPaneId`-keyed `Keymap::render_app_pane_bar_slots` (Phase 9 + Phase 12) — never via a typed `P::mode()` call — so each snapshot parameterizes on `FocusedPane`, not on the concrete pane type.

---

## What dissolves

- Every `KeyCode::*` direct match in input handlers.
- `App::enter_action`; `shortcuts::enter()` const fn.
- The seven hardcoded `Shortcut::fixed(...)` constants.
- The four per-context group helpers in `shortcuts.rs`.
- The threaded gating parameters in `for_status_bar`.
- `NAVIGATION_RESERVED` / `is_navigation_reserved`.
- `is_vim_reserved`'s hardcoded `VIM_RESERVED` table (replaced by reading `NavigationAction`'s bindings).
- The `+`/`=` parser collapse.
- `is_legacy_removed_action`.
- `InputContext` enum.

## What survives

- `Pane` trait — untouched. Bar refactor doesn't extend it.
- Per-pane host structs — untouched (gain a `Shortcuts` impl, lose nothing).
- `GlobalAction::Dismiss` — keeps `'x'`; gains `Esc` on `Toasts` only via `ToastsAction::Dismiss = [Esc, 'x']`.
- Vim-mode opt-in semantics — `h`/`j`/`k`/`l` still gated by `VimMode::Enabled`.

---

## CI tooling sanity check

Verify CI invocations operate on the intended scope before Phase 1 lands. Tools that walk `Cargo.toml` will see a `[workspace]` section they didn't see before — `cargo-mend`, `cargo-nextest` filters, format scripts, the nightly clean-build job. Each invocation needs a one-time check (does it operate on the binary only, the whole workspace, or both, and is that what we want?).

## Doctest + test infrastructure

> Full design in `test-infra.md`.

Summary:

- No doctests. Code blocks in `///` comments are ` ```ignore ` or prose.
- Private `test_support/` module (`pub(crate)`, `#[cfg(test)]`) for shared unit-test fixtures: `TestCtx`, `MockPane`, `MockTextInputPane`, `MockStaticPane`, `MockPairedNavPane`, no-op dispatchers, sample TOML strings, key-event constructors.
- Unit tests next to their module (`#[cfg(test)] mod tests`).
- Cross-module integration tests in `tui_pane/tests/` (5 files: `builder_full.rs`, `dispatch_routing.rs`, `bar_rendering.rs`, `vim_mode.rs`, `toml_errors.rs`) using only the public surface; each declares its own `IntegCtx` inline.
- No third `*-test-support` crate. Binary's `src/tui/tui_test_support.rs` stays separate; module rules enforce the boundary.

## Risks and unknowns

- **Workspace conversion.** Verified during Phase 1; no further action. Both crates build green, `cargo install --path .` still installs the binary, `Cargo.lock` and `target/` are unchanged in location.
- **`tui_pane` API under real use.** Designing a framework before its first client lands is speculative — trait signatures and builder methods may need revision once cargo-port consumes them. Mitigation: cargo-port is the first client; phases 5-6 will surface mismatches, and the framework can be revised before any external user touches it.
- **Scope precedence.** `NavigationAction::Right` and `ProjectListAction::ExpandRow` both default to `Right`. The "pane scope wins" rule is documented above and enforced by the input router. Lock with a unit test.
- **Settings toggle direction for booleans.** Today's `handle_settings_adjust_key` (`settings.rs:869-919`) inspects `KeyCode::Right` vs `Left` only for `SettingOption::CiRunCount` (a stepper); booleans flip regardless of direction. Plan splits into `ToggleNext` / `ToggleBack`. For booleans, both delegate to flip-the-bool. For the stepper, `ToggleNext` increments and `ToggleBack` decrements.
- **`is_vim_reserved` load order.** It must read `Navigation::defaults()` (constant builder), not the in-progress keymap, to avoid a load-order cycle when called inside `resolve_scope`. Defaults are constant and always available.
- **Framework grants `&mut Vec<Span>` to bar code.** Framework convention: each helper pushes only into vecs it owns content for. Reviewed at PR time.
- **Existing user TOML configs.** New scope names (`[finder]`, `[output]`, `[navigation]`, …) are additive; old configs without these tables still parse and use defaults. No breaking change.

---

## Definition of done

- Workspace exists with `tui_pane` member crate; binary crate consumes it.
- `tui_pane` exposes (every type is at the crate root — `tui_pane::Foo` flat, never `tui_pane::keymap::Foo`): `KeyBind`, `KeyInput`, `KeyParseError`, `Bindings<A>`, `bindings!`, `ScopeMap<A>`, `Keymap<Ctx>` + `KeymapBuilder<Ctx>`, `KeymapError`, `Pane<Ctx>`, `Shortcuts<Ctx>`, `Navigation<Ctx>`, `Globals<Ctx>`, `ShortcutState`, `Visibility`, `BarSlot<A>`, `BarRegion`, `Mode<Ctx>`, `Action` + `action_enum!`, `GlobalAction`, `VimMode`, `KeymapPane<Ctx>`, `SettingsPane<Ctx>`, `Toasts<Ctx>`, `SettingsRegistry<Ctx>`, `Framework<Ctx>`, `AppContext`, `FocusedPane`, `FrameworkPaneId`. The `__bindings_arms!` helper macro is `#[doc(hidden)]` but technically reachable as `tui_pane::__bindings_arms!` (a side-effect of `#[macro_export]`); it is not part of the supported surface.
- `ScopeMap::by_action: HashMap<A, Vec<KeyBind>>`; `display_keys_for(action) -> &[KeyBind]` exists; primary-key invariant locked.
- TOML parser accepts `key = "Enter"` and `key = ["Enter", "Return"]`; rejects in-array duplicates and cross-action collisions within a scope.
- `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction` exist in cargo-port. `ProjectListAction` has `ExpandRow` / `CollapseRow`.
- Every cargo-port app pane has `impl Shortcuts<App>`. `AppNavigation: Navigation<App>`. `AppGlobalAction: Globals<App>`.
- `App.framework: Framework<App>` field exists.
- Every input handler dispatches through the keymap; no `KeyCode::*` direct match for command keys remains.
- `NAVIGATION_RESERVED`, `is_navigation_reserved`, hardcoded `VIM_RESERVED`, the seven `Shortcut::fixed` constants, the four group helpers, `App::enter_action`, `shortcuts::enter()`, `InputContext` enum — all deleted.
- Framework owns the bar; cargo-port has zero bar-layout code.
- `make_app` hoisted to `src/tui/tui_test_support.rs`.
- Bar output for every focused-pane context is byte-identical to the pre-refactor bar under default bindings (snapshot-locked).
- All Phase 18 regression tests pass.

---

## Non-goals

- Not changing the `Pane` trait signature or any pane body's render code.
- Not unifying `PaneId::is_overlay()` semantics across the codebase — `InputContext` is being deleted, so the asymmetry resolves itself.
- Not making typed-character text input (Finder query, Settings numeric edit) keymap-driven — that's not what the keymap is for.
- Not extracting `FinderPane` into `tui_pane` in this refactor — left as a follow-up if it turns out to be reusable.
- Not migrating existing user TOML config files — old configs parse cleanly via the additive-table rule.
