# `tui_pane` library + universal keymap

## Principle

**Every command shortcut the user can rebind is bound through the keymap.** No `KeyCode::Enter` / `Up` / `Esc` / `Tab` / `Left` / `Right` matches remain for command dispatch. Structural preflights, modal confirmation, and text-input editing fallback may still inspect literal `KeyCode` / `Char(c)` values because those paths are not configurable shortcut commands. No literal `"Enter"` / `"Esc"` / `"Tab"` / `"↑/↓"` / `"+/-"` strings in any bar code. Every shortcut row in the bar comes from a binding lookup. Rebinding any command key updates the bar and the dispatcher in lockstep.

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
        ├── settings.rs             # SettingsRegistry;
        │                           #   add_bool / add_enum / add_int
        ├── framework.rs            # Framework<Ctx> aggregator;
        │                           #   mode_queries registry;
        │                           #   editor_target_path,
        │                           #   focused_pane_mode
        └── panes/                  # framework-internal panes
            ├── mod.rs
            ├── keymap.rs           # KeymapPane;
            │                       #   EditState::{Browse, Awaiting, Conflict}
            ├── settings.rs         # SettingsPane;
            │                       #   EditState::{Browse, Editing}
            └── toasts.rs           # Toasts<Ctx>; framework-owned typed pane
                                    #   (Phase 11 placeholder, Phase 12+ typed manager)
```

App-specific code stays in the binary crate. Framework code lives only in `tui_pane/src/`.

**Conceptual module dependencies** (Rust modules within one crate compile as a unit, so this is a readability layering, not a hard ordering):
- `keymap/` — bindings storage + traits + builder. The builder is the keymap's builder; it calls into `framework.rs` and `settings.rs` to file pane queries and settings during registration, but the resulting `Keymap<Ctx>` is the build product.
- `bar/` — reads `Keymap<Ctx>` and pane `Shortcuts<Ctx>` impls; emits `StatusBar`.
- `panes/` — framework panes implementing `Shortcuts<Ctx>`.
- `settings.rs` — `SettingsRegistry`.
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

Focus reads happen on `Framework<Ctx>` (`framework.focused()`); focus writes go through `AppContext::set_focus`. Framework code reads `framework.focused` directly without calling back through `Ctx`. The framework owns generic focus state and generic focus transitions. During the migration, cargo-port's `AppContext::set_focus` override mirrors framework focus into the legacy `Focus` subsystem; Phase 22 deletes that mirror and leaves the framework as the sole focus source.

The trait does **not** require `Ctx` to expose pane state — every pane's own state is reached via the per-pane dispatcher's free fn navigating through `Ctx` (`&mut ctx.panes.package`, etc.).

For the rest of this doc, signatures use `Ctx` (or `Ctx: AppContext`) when referring to the app context.

### Pane id design

Two enums + a wrapping type:

- `tui_pane::FrameworkOverlayId { Keymap, Settings }` — the framework's two overlay panes. Stored in `Framework::overlay: Option<FrameworkOverlayId>`. Phase 11 ships a unified `FrameworkPaneId { Keymap, Settings, Toasts }`; Phase 12 splits that into the overlay/focus pair so invalid states (e.g. `overlay = Some(Toasts)`, or `FocusedPane::Framework(Keymap)`) cannot be expressed.
- `tui_pane::FrameworkFocusId { Toasts }` — the framework's only directly-focusable pane. Toasts is Tab-focusable via the virtual cycle in `focus_step`; Keymap and Settings are overlay-only.
- `cargo_port::AppPaneId { Package, Git, ProjectList, … }` — cargo-port's 10 panes. Hand-written enum in `src/tui/panes/spec.rs` (today's enum, minus the framework variants).
- `tui_pane::FocusedPane<AppPaneId> { App(AppPaneId), Framework(FrameworkFocusId) }` — generic wrapper used in framework trait signatures. The binary uses this directly for focus tracking.

Linking the runtime tag to the compile-time pane type: every `Pane<App>` impl declares `const APP_PANE_ID: AppPaneId`. Calling `register::<PackagePane>()` records that value alongside the pane's dispatcher — registration populates the runtime mapping. The `AppPaneId` enum is the runtime side of the same registration.

Cargo-port's existing `tui::panes::PaneId` enum becomes a type alias `pub type PaneId = tui_pane::FocusedPane<AppPaneId>;` so existing call sites that name `PaneId` keep compiling; only the framework variants move out of the enum body.

### Ownership boundary

Framework-owned generic behavior: focus state, keyboard focus traversal, top-level mouse/click routing from screen position to focused pane, framework overlay open/close, framework panes (`SettingsPane`, `KeymapPane`, `Toasts`), keymap loading/dispatch, and the bar's region resolution/suppression/styling.

App-owned behavior: domain pane render state, pane-local row/domain hit geometry, domain action bodies, and app-specific panes such as Finder. App code supplies pane-local hit data to the framework boundary; it does not decide the top-level focus target and does not write focus directly. Focus mutation routes through `AppContext::set_focus`.


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

    /// List/cursor — the app's `Navigation` scope drives it (the
    /// framework reads keys through the `Navigation` trait's
    /// accessors); framework emits the `Nav` region. The default
    /// mode for app panes.
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
    const HOME:  Self::Actions;
    const END:   Self::Actions;
    fn defaults() -> Bindings<Self::Actions>;

    /// Translate a resolved navigation action into framework-owned
    /// `ListNavigation`. Default impl matches against the trait's
    /// `UP`/`DOWN`/`HOME`/`END` constants; returns `None` for
    /// `LEFT`/`RIGHT` (list panes don't consume horizontal moves)
    /// and any app-specific variants outside that set. The Phase 12
    /// dispatcher calls this when focus is
    /// `FocusedPane::Framework(FrameworkFocusId::Toasts)` to bridge
    /// from the binary's `Self::Actions` enum to the framework's
    /// `ListNavigation` vocabulary.
    fn list_navigation(action: Self::Actions) -> Option<ListNavigation> {
        if action == Self::UP { Some(ListNavigation::Up) }
        else if action == Self::DOWN { Some(ListNavigation::Down) }
        else if action == Self::HOME { Some(ListNavigation::Home) }
        else if action == Self::END { Some(ListNavigation::End) }
        else { None }
    }

    /// Free function the framework calls when any navigation action fires.
    /// `focused` lets the app dispatch to whichever scrollable surface
    /// owns the focused pane. One match arm per action, mirroring the
    /// `Shortcuts::dispatcher` and `Globals::dispatcher` pattern.
    fn dispatcher() -> fn(Self::Actions, FocusedPane<Ctx::AppPaneId>, &mut Ctx);
}
```

Pane scopes carry per-instance state and dispatch logic; their bar contribution depends on that state. `Navigation` has neither (no per-instance state) but it does need dispatch, since the focused pane needs to scroll on `Up`. The framework reads the six required consts (`UP`/`DOWN`/`LEFT`/`RIGHT`/`HOME`/`END`) to render the nav row and to apply vim-mode bindings, and calls `dispatcher()(action, focused, ctx)` to route. Phase 12 also adds a default-impl helper `list_navigation(action) -> Option<ListNavigation>` that the dispatcher uses to translate resolved navigation actions into framework-owned `ListNavigation` for focused-Toasts viewport scrolling.

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

Per Phase 9 review: `KeymapError` is the one error type spanning loader and builder validation; `BuilderError` was dropped from the design. Phase 9 ships `Keymap<Ctx>`; Phase 10 ships `KeymapBuilder<Ctx, State>` (typestate `Configuring → Registering`).

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


Each setting carries: TOML key, display label, value-getter closure, value-setter closure. Three value flavors covered: `bool`, `enum` (closed string set), `int` (with optional min/max).

The app provides only data + closures. It writes no `Shortcuts` impl, no mode state machine, no overlay rendering.

---

## `GlobalAction` — framework base, app extension


Framework owns pane-management, lifecycle, and the framework overlays:

```rust
// tui_pane
pub enum GlobalAction {
    Quit,
    Restart,
    NextPane,
    PrevPane,
    OpenKeymap,    // open framework's KeymapPane overlay
    OpenSettings,  // open framework's SettingsPane overlay
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
| `Dismiss`          | Runs framework dismiss chain: focused toast, then open framework overlay. If nothing dismissed, calls binary's `dismiss_fallback`. | `.dismiss_fallback(\|app\| -> bool { app.try_dismiss_focused_app_thing() })` |
| `NextPane`         | Pure pane-focus — framework knows the registered pane set.                         | (none — binary doesn't see this)           |
| `PrevPane`         | Pure pane-focus.                                                                   | (none)                                     |
| `OpenKeymap`       | Opens framework's `KeymapPane` overlay.                                            | (none)                                     |
| `OpenSettings`     | Opens framework's `SettingsPane` overlay.                                          | (none)                                     |

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

The dismiss chain rationale: framework-owned dismiss targets use one framework-owned path for both Esc and mouse-triggered dismiss. Splitting Esc-key dismiss between framework overlays and binary code duplicates ownership. App-level dismissables enter only through the one-fn `dismiss_fallback`.

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


Framework handles all TOML loading. Each registered scope's `SCOPE_NAME` constant drives table lookup; framework parses every recognized table, replaces that scope's bindings, leaves missing tables at their declared defaults+vim. App provides no TOML hooks.

### TOML errors

- In-array duplicates: `key = ["Enter", "Enter"]` → parse error.
- Cross-action duplicates within a non-globals scope (e.g. `[finder] activate = "Enter"` and `cancel = "Enter"`) → parse error (return `Err`).
- The `[global]` scope follows the per-action revert policy described under `GlobalAction` above — broken individual bindings revert to defaults; the loader returns `Ok` with a list of warnings.

The `ScopeMap::insert` `debug_assert` catches the same conditions for `defaults()` builders; the TOML loader returns them as real errors or warnings.


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
| `Toasts` (Phase 11 placeholder) | `Static` | `[(PaneAction, Single(Dismiss))]` |
| `Toasts` (Phase 12+ typed) | `Navigable` | `[(PaneAction, Single(Activate))]` once Phase 24 lands; nav slots from the app's `Navigation` scope (via the keymap) |

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
2. **Modal/text-input scope** — framework overlays (`KeymapPane` / `SettingsPane`) or the app-owned Finder pane get first claim on local keys. Toasts is *not* an overlay (`PaneId::is_overlay` excludes it today; same after the refactor).
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

`GlobalAction::Dismiss` is the single dismiss action — bound to `'x'` by default in cargo-port (`src/keymap.rs:409`). When focus is on Toasts, the dispatcher's `Dismiss` arm calls `dismiss_chain(ctx, fallback)` (Phase 12+), which calls `Framework::dismiss_framework(&mut self)`: that pops the focused toast through `Toasts::dismiss_focused()` first, falling through to `close_overlay()` and finally to the binary's optional `dismiss_fallback` hook.

The framework owns toast data (Phase 12+ typed `Toast` manager), so the dismiss path stays inside `tui_pane`. Binaries that want a different key for dismiss rebind `GlobalAction::Dismiss` via TOML. The bar renders `GlobalAction::Dismiss` in the global region while focused on Toasts; the pane-action region renders any nav / activation rows from `ToastsAction`.

---

## Bar render — concrete dispatch

`render.rs:531-558` today calls `app.input_context()`-driven `for_status_bar`. Post-deletion, the framework call dispatches off `app.framework().focused()` plus `app.framework().overlay()`. App panes flow through `FocusedPane::App(id)`, Toasts through `FocusedPane::Framework(FrameworkFocusId::Toasts)`, and Settings / Keymap through the overlay layer.

The `Settings` / `Keymap` / `Toasts` panes use their internal mode flags (Browse/Editing, Browse/Awaiting/Conflict, etc.) to vary `bar_slots` and `shortcut` output. The current `InputContext::SettingsEditing` / `KeymapAwaiting` / `KeymapConflict` arms collapse into pane-internal mode dispatch.

`overlay_editor_target_path` (`input.rs:413`) becomes `app.framework.editor_target_path()` — Settings and Keymap panes each expose `fn editor_target(&self) -> Option<&Path>`; framework chooses based on the open overlay.

---

## Phases

Each phase is a single mergeable commit. Each commit must build green and pass `cargo nextest run --workspace`. No sub-phases (`Na/Nb/Nc`) — every increment gets its own integer.

### Current state after Phase 10

Phases 1–10 are complete. The shipped surface now includes the workspace crate, key/action/binding primitives, flat crate-root exports, `Pane` / `Shortcuts` / `Navigation` / `Globals`, `Framework<Ctx>` skeleton plus lifecycle dispatch, the post-reset `Keymap<Ctx>` boundary (`dispatch_app_pane`, `render_app_pane_bar_slots`, `key_for_toml_key`), typestate `KeymapBuilder`, typed `navigation` / `globals` singleton storage, framework globals, settings registry, TOML overlay, and vim extras.

The remaining work starts at Phase 11. Do not reintroduce reset-removed surfaces (`scope_for`, `scope_for_typed`, public erased traits, `PendingEntry`, TypeId primary indices). Production construction is `build_into(&mut framework)`; `build()` is only for tests that do not query `Framework::focused_pane_mode`.

### Remaining architecture review before Phase 11

The remaining architecture needs a tightening pass before implementation. These are not Phase-10 code bugs; they are places where the plan still assumes surfaces that either do not exist after the Phase 9 reset or will not compose cleanly with real cargo-port state.

1. **`Shortcuts` should not require owned pane instances in the keymap.** The shipped builder stores `pane: P` inside `PaneScope<Ctx, P>`, and `Shortcuts::{visibility,state,bar_slots}` take `&self`. Cargo-port pane structs are stateful render/hit-test owners (`Viewport`, caches, pollers, row rects), so registering a second pane instance in `Keymap` duplicates state and invites stale reads. Before Phase 14, either make these `Shortcuts` methods associated functions (`fn visibility(action, ctx)`, `fn state(action, ctx)`, `fn bar_slots(ctx)`) and return to type-only registration, or introduce explicit zero-state adapter types for shortcut scopes. Type-only associated functions match the existing dispatcher/mode design better.

2. **Framework-pane handlers must avoid the `&mut Framework` + `&mut Ctx` split-borrow trap.** The Phase 11 surface says `KeymapPane::handle_key(&mut self, ctx: &mut Ctx, ...)`, `SettingsPane::handle_key(&mut self, ctx: &mut Ctx, ...)`, and `Toasts::handle_key(&mut self, ctx: &mut Ctx, ...)`. If the pane is stored inside `ctx.framework_mut()`, calling that method while also passing `&mut ctx` will not compile without take/replace or interior mutability. Prefer command-returning pane methods that only mutate pane-local state while borrowed, then apply the returned command to `Ctx` after the pane borrow ends. Free dispatcher functions can orchestrate the borrow scopes.

3. **Framework-pane access to keymap/settings data is under-specified.** The settings registry currently lives on `Keymap<Ctx>`, while Phase 11 puts `SettingsPane` on `Framework<Ctx>`. `KeymapPane` also needs keymap metadata and mutation/persistence support. Decide ownership before implementing panes: either keep registries/metadata on `Keymap` and pass `&Keymap` into framework-pane operations, or transfer the settings registry into `Framework` during `build_into`. The current plan gives framework panes neither a clean read path nor a clean mutation path.

4. **`RenderedSlot` is too flat for the planned bar.** It carries one `key` and one `label`, so `RuntimeScope::render_bar_slots` cannot represent `BarSlot::Paired(left, right, shared_label)` and drops alternate bindings for `Single(action)`. That conflicts with ProjectList rows (`←/→ expand`, `+/- all`, `→/l`) and with the original multi-bind bar requirement. Before Phase 13, replace `RenderedSlot` with a resolved slot enum such as `Single { keys: Vec<KeyBind>, label, state }` / `Paired { left_key, right_key, label, state }`, or otherwise carry enough fields for region renderers to format paired and multi-key rows correctly.

5. **App dispatch order must keep pane scope before navigation.** The Phase 11 dispatch chain currently says framework globals → app globals → navigation → per-pane scope. That breaks the documented precedence where `ProjectListAction::ExpandRow` wins over `NavigationAction::Right`. The app-pane branch should be framework globals → app globals → focused pane scope → navigation → unhandled.

6. **Text-input mode should not blanket-suppress `PaneAction` rows.** Phase 13 currently suppresses `PaneAction` for `Mode::TextInput(_)`, but Settings Editing and Keymap Awaiting still need local actions like Cancel/Confirm visible. Text input should suppress `Nav` and usually `Global`; the focused pane should decide whether it has `PaneAction` slots by returning them from `bar_slots`.

7. **Primary-key reverse lookup is not enough for structural checks.** `key_for_toml_key(id, action)` returns one primary key. Phase 18 uses it to decide whether the inbound key should clear output, but multi-bind actions can have several keys. Add a predicate (`is_key_bound_to_toml_key`) or all-key getter (`keys_for_toml_key`) before using this for structural preflight or the keymap overlay.

8. **The framework-owned keymap overlay needs registered metadata and mutation APIs.** The plan says the binary supplies `(P::APP_PANE_ID, P::Actions::ALL)` pairs, but `KeymapPane` is moving into `tui_pane`. Better: collect scope/action metadata during `register::<P>` while `P::Actions::ALL` is typed, store an erased metadata table on `Keymap`, and expose framework-owned rebind operations that update the live scope and persistence target. Otherwise Phase 11/14 will have to reach back into binary-specific keymap UI logic.

One stale-plan cleanup item folds into the same pass: Phase 23's TOML/vim test currently contradicts the shipped builder (vim extras apply after TOML overlays).

### Phase 1 — Workspace conversion ✅

Convert `cargo-port-api-fix` into a Cargo workspace.

Concrete steps:

1. Root `Cargo.toml` keeps `[package]` (binary) and adds `[workspace] members = ["tui_pane"]` + `resolver = "3"` (resolver must be explicit; not inferred from edition 2024 in workspace context).
2. Promote the existing `[lints.clippy]` and `[lints.rust]` blocks verbatim to `[workspace.lints.clippy]` / `[workspace.lints.rust]` (including `missing_docs = "deny"` from day one). Root `[lints]` becomes `workspace = true`.
3. Create `tui_pane/` as a sibling directory (not `crates/tui_pane/`) with `Cargo.toml` (`crossterm`, `ratatui`, `dirs` deps; `[lints] workspace = true`) and `src/lib.rs` carrying crate-level rustdoc.
4. Add `tui_pane = { path = "tui_pane", version = "0.0.4-dev" }` to the binary's `[dependencies]`.
5. Apply the CI flag updates (`cargo +nightly fmt --all`, `cargo mend --workspace --all-targets`, `cargo check --workspace` in the post-tool-use hook). These can ship in a separate prior commit since they're no-ops on the current single-crate layout.
6. Update auto-memory `feedback_cargo_nextest.md` to clarify default `cargo nextest run` only tests the root package; iteration loops should pass `-p` or `--workspace`. `feedback_cargo_install.md` is unchanged (the binary stays at root).

After Phase 1: `cargo build` from the root builds both crates; `cargo install --path .` still installs the binary; `Cargo.lock` and `target/` stay at the workspace root.

**Per-phase rustdoc precondition.** Phases 2–17 add `pub` items to `tui_pane`. Each pub item ships with a rustdoc summary line — `missing_docs = "deny"` is workspace-wide from Phase 1, so a missing doc breaks the build. Module headers (`//!` blocks) must use the format **one-line summary, blank `//!`, then body** — `clippy::too_long_first_doc_paragraph` (nursery) rejects multi-sentence opening paragraphs (Phase 3 retrospective surfaced this).

### Phases 2–10 — `tui_pane` foundations

Phases 2–10 land the entire `tui_pane` public surface in dependency order, one mergeable commit per phase. Each phase below carries the type signatures, error variants, and contracts that subsequent phases depend on.

**Strictly additive across Phases 2–10.** Nothing moves out of the binary in this group. The binary continues to use its in-tree `keymap_state::Keymap`, `shortcuts::*`, etc., untouched. The migration starts in Phase 14.

**Pre-Phase-2 precondition (post-tool-use hook).** Decide hook strategy before Phase 2 lands: repo-local override at `.claude/scripts/hooks/post-tool-use-cargo-check.sh` adding `--workspace`, vs. updating the global script at `~/.claude/scripts/hooks/post-tool-use-cargo-check.sh`. Without the flag, edits to `tui_pane/src/*.rs` from inside the binary working dir will not surface `tui_pane` errors. Repo-local override is the lower-blast-radius option.

**README precondition (Phase 10).** `tui_pane/README.md` lands at the end of Phase 10 — when the public API is complete. It covers crate purpose + a minimal example using `Framework::new(initial_focus)`. Code blocks in the README are ` ```ignore ` (no doctests in this crate).

### Phase 2 — Keys ✅

Add `tui_pane/src/keymap/key_bind.rs` (`KeyBind`, `KeyInput`, `KeyParseError`). Leaf types — nothing else in `tui_pane` depends on them yet.

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
- `KeyBind::shift` / `ctrl` were respec'd to take `impl Into<Self>` (i.e. `impl Into<KeyBind>`) rather than `impl Into<KeyCode>`. Reason: crossterm's `KeyCode` does not implement `From<char>`, so the planned `impl Into<KeyCode>` bound rejects `KeyBind::shift('g')`. Taking `Into<KeyBind>` reuses the three `From` impls and makes `shift`/`ctrl` composable (`KeyBind::ctrl(KeyBind::shift('g'))` → CTRL|SHIFT).
- `KeyParseError` ships with 3 variants (`Empty`, `UnknownKey`, `UnknownModifier`) — `InvalidChar` was dropped because no parser path emits it.
- Parser supports `"Control"` as a synonym for `"Ctrl"` (both produce `KeyModifiers::CONTROL`); `"Space"` parses to `KeyCode::Char(' ')`. Neither was called out in the plan.

**Surprises:**
- `KeyCode` has no `From<char>` impl in crossterm — and orphan rules block adding one. This forced the `impl Into<Self>` rework.
- Modifier display order (`Ctrl` → `Alt` → `Shift`) and the case-preservation policy in `parse` (`"Ctrl+K"` → `Char('K')`, not `Char('k')`) are now baked into Phase 2 tests. Phase 9 (TOML loader) inherits both as facts; if the loader needs case-insensitive letter lookup, that is a *keymap-layer* normalization, not a `KeyBind::parse` concern.

**Implications for remaining phases:**
- Phase 9 (`Keymap<Ctx>` + TOML loader) must decide letter-case normalization policy explicitly — `parse` preserves case as-is.
- Future framework error types (`KeymapError` Phase 4 skeleton, fill in Phase 9) should use `#[derive(thiserror::Error)]` with `#[from] KeyParseError` for source chaining, per the pattern established here.

#### Phase 2 Review

- Phase 3: rename `keymap/traits.rs` → `keymap/action_enum.rs` so the file name matches its sole resident (`Action` + `action_enum!`) and does not collide with Phase 7's per-trait file split.
- Phase 4: `KeymapError` ships with `#[derive(thiserror::Error)]` + `#[from] KeyParseError` for source chaining, and unit tests are rescoped to constructs that exist by end of Phase 4 (vim-application test deferred to Phase 10). `bindings!` macro tests now cover composed `KeyBind::ctrl(KeyBind::shift('g'))`.
- Phase 9: loader explicitly lowercases single-letter TOML keys (so `quit = "Q"` binds `Char('q')`); modifier display order is canonical `Ctrl+Alt+Shift+key` (no round-trip ordering preservation); vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods); `KeymapError` source chain from `KeyParseError` is asserted.
- Phase 13: paired-row separator policy made explicit — `Paired::debug_assert!` covers only the parser-producible `KeyCode` set; exotic variants may panic, and widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep.
- Phase 14: `anyhow = "1"` lands in the binary's `Cargo.toml` here (first call site that needs context wrapping is `Keymap::<App>::builder(...).load_toml(?)?.build()?`).
- §1 (`Pane id design`): `PaneId` → `FrameworkPaneId` everywhere, including the inside-the-crate short form, so the type name is one-to-one across library and binary call sites.
- Phase 2 shipped: `shift`/`ctrl` take `impl Into<Self>`, `From<KeyEvent>` documented, 3-variant `KeyParseError` (`InvalidChar` dropped), parser policy (`"Control"` synonym, `"Space"` token, case-preserving) locked.
- TOML loader follows the Zed/VSCode/Helix-aligned letter-case decision: loader lowercases single-letter ASCII keys (`"Q"` → `Char('q')`, never `Shift+q`); modifier tokens are case-insensitive on input but writeback canonical capitalized; named-key tokens (`Enter`, `Tab`, `F1`, …) are case-sensitive with no aliases; non-ASCII letters not lowercased; modifier repeats silently OR'd (not rejected — bitwise OR is idempotent).
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
- Phase 4 lands the real `impl Display` for `KeymapError` via `#[derive(thiserror::Error)]` per the Phase 2 retrospective decision.
- Phase 4 (`bindings!` macro) follows the same `#[macro_export] macro_rules!` declaration template used here; the doctest pattern can mirror Phase 3's approach (`crate::action_enum! { … }` inside an internal `mod tests`).
- Phase 14 (binary swap to `tui_pane::action_enum!`): seven existing `action_enum!` invocations in `src/keymap.rs` swap to the `tui_pane::` prefix; the macro's grammar is identical, so each invocation needs only the prefix change.

#### Phase 3 Review

Architectural review of remaining phases (4-17) returned 18 findings — 13 minor (applied directly), 5 significant (decided with the user). Resolved outcomes:

- **Renamed `keymap/base_globals.rs` → `keymap/global_action.rs`** so the file name matches the contained type (`GlobalAction`). User did the file rename in their editor; doc references and `mod.rs` synced. No `BaseGlobals` type ever existed; the "base" prefix earned nothing and broke the established `key_bind.rs → KeyBind` convention.
- **Phase 9 anchor type:** `Keymap<Ctx>` lives in `keymap/mod.rs` (option c). Workspace lint `self_named_module_files = "deny"` rules out `keymap.rs` + `keymap/` sibling layout, and `clippy::module_inception` rules out `keymap/keymap.rs`. Phase 6 already follows the same convention with `framework/mod.rs` holding `Framework<Ctx>`. Plan's prior `keymap/mod_.rs` was a typo.
- **Framework owns `GlobalAction` dispatch (significant pivot, item 2):** `KeymapBuilder` no longer takes positional `(quit, restart, dismiss)` callbacks. Framework dispatches all seven variants:
  - `Quit` / `Restart` set `Framework<Ctx>::quit_requested` / `restart_requested` flags; binary's main loop polls.
  - `Dismiss` runs framework chain (focused toast → open framework overlay), then bubbles to optional `dismiss_fallback`.
  - `NextPane` / `PrevPane` / `OpenKeymap` / `OpenSettings` framework-internal as before.
  - Binary opts in via optional `.on_quit()` / `.on_restart()` / `.dismiss_fallback()` chained methods on `KeymapBuilder`.
  - Rationale: framework-owned dismiss targets use one framework-owned path for both Esc and mouse-triggered dismiss. Splitting Esc-key dismiss between framework overlays and binary code duplicates that ownership.
  - Touches Phase 6 (Framework skeleton +2 fields, +2 methods), Phase 10 (KeymapBuilder drops 3 args, gains 3 chained hooks), Phase 11 (Toasts dismiss participation, `Framework::dismiss()` method), Phase 19 (binary main loop polls flags, deletes `Overlays::should_quit`).
- **Cross-enum `[global]` collision = hard error (item 3):** `KeymapError::CrossEnumCollision { key, framework_action, app_action }` at load time. Definition-time error — app dev renames their colliding `AppGlobalAction::toml_key` string. Per-binding revert policy still handles user typos.
- **`GlobalAction::defaults()` lives on the enum (item 4):** `pub fn defaults() -> Bindings<Self>` lands in Phase 4 (when `Bindings` + `bindings!` exist) inside `global_action.rs`. Loader and builder consume it.
- **Cross-crate macro integration test (item 5):** `tui_pane/tests/macro_use.rs` lands as a Phase 3 follow-up — exercises `tui_pane::action_enum!` from outside the crate. Phase 4 extends it for `tui_pane::bindings!`.

Minor findings applied directly (no user gating):
- Phase 4 root re-exports (`Bindings`, `KeyBind`) called out for the `bindings!` macro's `$crate::` paths.
- `KeymapError` variant set spelled out in Phase 4 (with `#[derive(thiserror::Error)]`).
- `Shortcuts::vim_extras() -> &'static [(Self::Actions, KeyBind)]` called out in Phase 7 with default `&[]`.
- Vim-mode skip-already-bound test moved Phase 9 → Phase 10 (vim application is the builder's job per "Vim mode — framework capability" §).
- `AppContext::AppPaneId: Copy + Eq + Hash + 'static` super-trait set added to Phase 6 (required by Phase 11's `HashMap<AppPaneId, fn(&Ctx) -> Mode<Ctx>>`).
- `Action` super-trait set is `Copy + Eq + Hash + Debug + Display + 'static` (adds `Debug` + `Display` over the original spec).
- Phase 9 explicit "loader lifts `None` from `from_toml_key` into `KeymapError::UnknownAction`" wording added.
- `clippy::too_long_first_doc_paragraph` (nursery) guidance added to the per-phase rustdoc precondition.
- `pub use keymap::GlobalAction;` at crate root noted in Phase 14.
- Paired-row separator policy in Phase 13 shortened to a one-line cross-reference of Phase 2's locked decision.

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

**Cross-crate macro integration test.** Extend `tui_pane/tests/macro_use.rs` (the scaffolding lands as a Phase 3 follow-up exercising `action_enum!` only) to add a `bindings!` invocation. Both macros are compiled here from outside the defining crate — `#[macro_export]` + `$crate::` paths are easy to break under cross-crate use, and this test locks the public path before Phase 14's binary swap depends on it.

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
- **Every Phase 5+ module declaration must be `mod foo;`** (not `pub mod foo;`) at every level. Affects Phase 5 (`bar/region.rs`, `bar/slot.rs`), Phase 6 (`framework/`), Phase 7 (scope traits), Phase 9 (`keymap/container.rs` or wherever `Keymap<Ctx>` lands), Phase 10 (`keymap/builder.rs`), Phase 11 (`panes/*`), Phase 13 (`bar/render.rs`).
- **Every `tui_pane::keymap::*` path in design docs is now stale.** `tui-pane-lib.md` needs a sweep: `crate::keymap::Foo` → `crate::Foo` (and `tui_pane::keymap::Foo` → `tui_pane::Foo` in public-API examples).
- **Phase 14 binary swap uses flat paths.** `use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`. Every file in `src/tui/` that touches keymap types will see this.
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
- **Phase 13 (Bar renderer)** gains: `pub use bar::StatusBar;` plus standing-rule 1 reminder.
- **Phase 14 (App swap)** gains: flat-namespace import note (`use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`) and binary-side `mod` rule reminder.
- **New "Phase 5+ standing rules" subsection** added after the Phase 4 retrospective: locks the seven standing rules (private `mod`, flat re-exports, `pub(super)` for framework-internal, `#[must_use]` on getters, flat `$crate::*` macro paths, new `#[macro_export]` extends `tests/macro_use.rs`, `cargo mend --fail-on-warn` as phase-completion gate).
- **Definition of done** rewritten to enumerate every public type at crate-root flat paths and to call out `__bindings_arms!` as `#[doc(hidden)]` but technically reachable.
- **Spec sweep:** stale `crate::keymap::<submod>::Foo` sub-paths replaced with the facade-path form `crate::keymap::{Foo, ...}`; explanatory comments added about why the public API is flat.
- **Reviewed and not changed:** `tui_pane/README.md` deferred to Phase 19 (subagent finding #20 — no earlier baseline justified). `bind_many` requiring `A: Clone` (subagent finding #10 — auto-satisfied because `Action: Copy`, no plan change needed).

These apply to every remaining phase without further mention; phase blocks below assume them. Restate only where a phase has a specific exception.

1. **Module declarations are `mod foo;`** at every level — never `pub mod foo;`. Parents expose the API via `pub use foo::Type;` re-exports. `cargo mend` denies `pub mod` workspace-wide, including the binary side (`src/tui/...` in Phase 13).
2. **Public types live at the crate root.** Every `tui_pane` public type re-exports from `tui_pane/src/lib.rs` so callers write `tui_pane::Foo` (flat). The `tui_pane::keymap::*` namespace does not exist publicly.
3. **Framework-internal construction is `pub(super)`.** New / insert / build methods that only the framework's own siblings call use `pub(super)`, never `pub(crate)`. Project memory `feedback_no_pub_crate.md` for rationale.
4. **Public getters get `#[must_use]` pre-emptively.** Clippy `must_use_candidate` (pedantic, denied) fires on every getter that returns a value the caller can ignore.
5. **Macros use flat `$crate::*` paths.** Every `#[macro_export]` macro references re-exported root types: `$crate::Bindings`, `$crate::KeyBind`, `$crate::Action`. Never `$crate::keymap::Foo`.
6. **New `#[macro_export]` extends `tests/macro_use.rs`.** Cross-crate path stability is locked by that file; any new exported macro adds an invocation there.
7. **Phase-completion gates.** `cargo build`, `cargo nextest run`, `cargo +nightly fmt`, `cargo clippy --workspace --all-targets`, `cargo mend --fail-on-warn` — all clean before the phase is marked ✅.
8. **Every new pub item gets a doc comment; every new module gets a `//!` header.** Module `//!` explains what lives in the file and why; type `///` explains the role; method `///` explains what callers get back; variant `///` explains the case. One-liners are fine where the name carries the meaning. The Phase 5 files (`bar/region.rs`, `bar/slot.rs`, `bar/mod.rs`) and Phase 3's `keymap/action_enum.rs` / `keymap/global_action.rs` are the reference baseline — match that density.
9. **Public `&self` value-returning methods carry both `#[must_use]` and `const fn`.** Setters (`&mut self`) carry `const fn` when the body is const-eligible (Rust 1.83+ permits `&mut` in const fn). Clippy nursery `missing_const_for_fn` is denied workspace-wide and fires on every getter / setter that could be const. Phase 6's `Framework<Ctx>` getters (`focused`, `set_focused`, `quit_requested`, `restart_requested`) are the reference baseline; `Framework::new` itself drops `const fn` at Phase 10 once `HashMap::new()` enters the body.

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

Leaf types only — the renderer that consumes them lands in Phase 13.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::{BarRegion, BarSlot, Mode, ShortcutState, Visibility};`. `bar/mod.rs` is `mod region; mod slot; pub use region::BarRegion; pub use slot::{BarSlot, ShortcutState, Visibility}; pub use ...Mode;` (or wherever `Mode` lands).

**No `Shortcut` wrapping struct.** Phase 7's `Shortcuts<Ctx>` trait splits the bar-entry payload across two orthogonal axes: `fn visibility(&self, action, ctx) -> Visibility` (default `Visible`, `Hidden` removes the slot) and `fn state(&self, action, ctx) -> ShortcutState` (default `Enabled`, `Disabled` grays the slot). The label is static (`Action::bar_label()`); there is no per-frame label override.

**`action_enum!` grammar amendment.** The macro arm changes from `Variant => "key", "desc";` to `Variant => ("key", "bar", "desc");`. Phase 3's existing `action_enum!` invocations in the keymap module and the `tests/macro_use.rs` smoke test must be updated in this phase. The 12-arm cargo-port migration in Phase 14 inherits the new grammar. The hand-rolled `GlobalAction` `Action` impl shipped in Phase 3 also needs a `bar_label()` method body — one match arm per variant (`Quit => "quit"`, `Restart => "restart"`, etc.).

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
- **Phase 14 cargo-port `action_enum!` migrations need the third positional string.** Every existing app-side invocation gains a bar label between the toml key and description. For app actions where the bar text matches the toml key, just duplicate the literal — no design decision per arm.
- **Phase 13 bar renderer reads `BarRegion::ALL` for layout order.** Already reflected in trait def — `Vec<(BarRegion, BarSlot<Self::Actions>)>` returned, renderer groups by region.
- **No new public types added to `tui_pane::*` beyond the announced bar primitives** (`BarRegion`, `BarSlot`, `Mode`, `ShortcutState`, `Visibility`). Every later-phase reference to `tui_pane::Shortcut` (the deleted wrapping struct) is dead — caught any in Phase 5's plan-doc sweep, but Phase 7 implementers should not pattern-match on `Shortcut` in muscle memory.

#### Phase 5 Review

- **Phase 7 (Scope traits)** plan body now enumerates the full `Shortcuts<Ctx>` method set and explicitly states the `label` / `state` default bodies leveraging `Action::bar_label` and `ShortcutState::Enabled`.
- **Phase 7** also explicitly states `Globals<Ctx>` has no `bar_label` method, and adds a `Shortcut` (singular wrapping struct) doc-grep step to confirm zero residue.
- **Phase 9 (Keymap container)** plan gains a one-line clarification that `bar_label` is code-side only — the TOML loader never reads or writes it.
- **Phase 13 (Bar renderer)** plan now states the per-region `Mode` suppression rules in line with shipped `bar/mod.rs` docstrings (`Static` suppresses `Nav`, `TextInput(_)` suppresses `Nav` + `PaneAction` + `Global`).
- **Phase 14 (App swap)** gains an explicit migration-cost callout that every existing `action_enum!` invocation in `src/tui/` needs a third positional `bar_label` literal.
- **Phase 23 (Regression tests)** reworded to assert each global slot's bar text comes from `action.bar_label()`, not a `Globals` trait method.
- **Visibility sync:** `ScopeMap::new`/`insert` migrated from `pub(crate)` → `pub(super)` to match shipped code (Phase 4 retrospective decision; finalized here per post-phase doc-sync rule).
- **Reviewed and not changed:** `Globals::render_order` (subagent finding #6 — plan unchanged); binary-side `pub mod` audit in Phase 13 (subagent finding #11 — grep of `src/tui/**/*.rs` found zero `pub mod`, no audit needed); `__bindings_arms!` cross-crate test (subagent finding #10 — `#[doc(hidden)]` is supported-surface-out, not worth dedicated test); `set_focused` consistency (subagent finding #4 — already consistent); Phase 10 builder-level cross-crate test (subagent finding #15 — Phase 10 already lists end-to-end builder tests).

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
- **Phase 14** adds an `impl AppContext for App` line item with a note that `set_focus` defaults out — only `framework()` / `framework_mut()` are required.
- **Phase 23 regression suite** adds a "set_focus is the single funnel" test: an override impl that counts calls observes every framework focus change.

**Reviewed and not changed:**
- Finding #6 (macro-emitted const fn): user feedback — const is opportunistic, clippy gates it; do not escalate const-eligibility as a finding requiring approval (saved as `feedback_const_opportunistic.md`).

### Phase 7 — Scope traits ✅

> **Note on shipped vs. described surface.** Phase 7's actual code commit (`8f657cc`) shipped a **pre-redesign** form of the scope traits: `Shortcuts<Ctx>: 'static` (no `Pane` supertrait), `type Variant`, `fn label(&self, …) -> Option<&'static str>`, `fn input_mode() -> fn(&Ctx) -> InputMode`, and the `InputMode { Static, Navigable, TextInput }` enum. The deliverables list below describes the **post-redesign** surface adopted by the doc-sweep commit (`5cacb7b`) — `Pane<Ctx>` supertrait, `type Actions`, `Mode<Ctx>` with handler-in-variant, `Visibility` axis. **Phase 8 brings code into alignment with this description.** Until Phase 8 lands, the on-disk code lags the doc by exactly that delta — intentional, recorded here so a reader who diffs Phase 7 deliverables against `tui_pane/src/keymap/{shortcuts,navigation,globals}.rs` is not surprised.

**Cross-crate test fixtures must use multi-variant enums.** Phase 7 adds `Pane<Ctx>` / `Shortcuts<Ctx>` / `Navigation<Ctx>` / `Globals<Ctx>` smoke tests in `tests/macro_use.rs` (standing rule 6). Per the Phase 6 retrospective surprise: single-variant test enums emit `dead_code` because the lint ignores derived impls. Use multi-variant fixtures (e.g. `CrossCrateNavAction::{Up, Down, Left, Right}`); if a single-variant fixture is unavoidable, gate the unused variant with `#[allow(dead_code, reason = "...")]`.

Files (one per trait — each is independent, the heaviest is `Shortcuts<Ctx>` with 6 methods + 1 const + 1 assoc type):

- `tui_pane/src/pane.rs` — `Pane<Ctx>` with `const APP_PANE_ID: Ctx::AppPaneId` and `fn mode() -> fn(&Ctx) -> Mode<Ctx>` (default `|_| Mode::Navigable`). The supertrait for every per-pane trait. The framework registry stores the returned `mode` pointer keyed by `AppPaneId`; pane-internal callers write `Self::mode()(ctx)`.
- `tui_pane/src/keymap/shortcuts.rs` — `Shortcuts<Ctx>: Pane<Ctx>` with `type Actions: Action;` and method set: `defaults`, `visibility`, `state`, `bar_slots`, `vim_extras`, `dispatcher`, plus `SCOPE_NAME` const. Default `visibility` returns `Visibility::Visible`; default `state` returns `ShortcutState::Enabled`; default `bar_slots` emits `(PaneAction, Single(action))` per `Self::Actions::ALL` in declaration order. Per-pane impls override only when one of these axes is state-dependent. The bar **label** is always `Action::bar_label()` from `action_enum!` — there is no per-frame label override on the trait. `vim_extras() -> &'static [(Self::Actions, KeyBind)]` defaults to `&[]` (cargo-port's `ProjectListAction` overrides for `'l'`/`'h'` in Phase 14).
- `tui_pane/src/keymap/navigation.rs` — `Navigation<Ctx>` with `type Actions: Action;`.
- `tui_pane/src/keymap/globals.rs` — `Globals<Ctx>` with `type Actions: Action;` (app-extension globals, separate from the framework's own `GlobalAction` from Phase 3). The trait has **no** `bar_label(action) -> &'static str` method — Phase 5's `Action::bar_label` (live on every action enum, including the macro-generated and the hand-rolled `GlobalAction`) is the single source. Bar code calls `action.bar_label()` regardless of scope.

`keymap/action_enum.rs` holds the `Action` trait and the `action_enum!` macro.

**`Shortcut` wrapping struct is dead.** Phase 5 split it into orthogonal `Visibility` and `ShortcutState` axes. Phase 7 prep verified no `Shortcut\b` (singular wrapping struct) references remain — `Shortcuts`, `ShortcutState`, `Visibility` are the only valid forms.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use pane::{Pane, Mode};` and `pub use keymap::{Shortcuts, Navigation, Globals};`. `keymap/mod.rs` adds `pub use shortcuts::Shortcuts; pub use navigation::Navigation; pub use globals::Globals;`. Inner files declare `mod shortcuts; mod navigation; mod globals;` (private — standing rule 1).

**Implications for later phases (locked here):**
- **Phase 9 `Keymap<Ctx>` container** relies on `Shortcuts::SCOPE_NAME`, `Navigation::SCOPE_NAME` (defaults to `"navigation"`), `Globals::SCOPE_NAME` (defaults to `"global"`) for TOML table dispatch. The default-impl test in `globals.rs` confirms `Globals<TestApp>::SCOPE_NAME == "global"` so the `[global]` table can carry both framework `GlobalAction` and the app's `Globals` impl simultaneously. Build entry point is `KeymapBuilder::build_into(&mut Framework<Ctx>) -> Result<Keymap<Ctx>, KeymapError>`. Binary constructs `Framework::new(initial_focus)` first, then hands it to the builder. The registry write is a single locus.
- **Phase 10 builder** populates the framework's per-pane registries by walking `Pane::mode()` for each registered `P: Pane<Ctx>` and storing the returned `fn(&Ctx) -> Mode<Ctx>` keyed by `P::APP_PANE_ID`. Because `mode` is a free fn returning a bare `fn` pointer, the builder needs only `P` as a type parameter — never a typed `&P` instance. Standing rule 9's `const fn` clause applies to inherent methods only — trait-default bodies can't be `const fn` in stable Rust. `const fn` with `&mut self` requires Rust ≥ 1.83 (verified before the `pub(super) const fn request_quit/request_restart` setters land).
- **Phase 11 `Framework<Ctx>::focused_pane_mode`** dispatches through the registry without holding a typed `&PaneStruct`. The default `|_ctx| Mode::Navigable` is what panes that don't override fall back to. `Framework<Ctx>::mode_queries` is private; the only writer is `pub(super) fn register_app_pane(&mut self, id: Ctx::AppPaneId, query: fn(&Ctx) -> Mode<Ctx>)`. Framework panes do not impl `Shortcuts<Ctx>` (the trait requires `APP_PANE_ID: Ctx::AppPaneId`, which framework panes lack); each framework pane (`KeymapPane`, `SettingsPane`, `Toasts`) ships inherent `defaults() / handle_key() / mode() / bar_slots()` methods directly on the struct; bar renderer + dispatcher special-case `FocusedPane::Framework(_)`. Framework pane input handling is inherent `pub fn handle_key(&mut self, ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome` for `KeymapPane` / `SettingsPane` (overlay panes need `&mut Ctx` to mutate app state), and `pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome` for `Toasts<Ctx>` from Phase 12 onward (pure pane-local viewport mutation; the focused-Toasts dispatch chain reaches Ctx via `on_navigation` / `try_consume_cycle_step` / `handle_key_command` instead). All three return `KeyOutcome::{Consumed, Unhandled}`.
- **Phase 13 (bar render)** writes region-suppression rules in terms of `framework.focused_pane_mode(ctx)` rather than `P::mode()(ctx)` — the renderer holds a `FocusedPane`, not a typed `P`. The bar renderer calls `Keymap::render_app_pane_bar_slots(id, ctx)` and the input dispatcher calls `Keymap::dispatch_app_pane(id, &bind, ctx)`; both are `AppPaneId`-keyed and consume `RenderedSlot` / `KeyOutcome` directly. The crate-private `RuntimeScope<Ctx>` trait (renamed from `ErasedScope`) carries the per-pane vtable but is invisible to external callers — they go through the three concrete public methods on `Keymap<Ctx>`.
- **Phase 14 (mode override)** closure body: `fn mode() -> fn(&App) -> Mode<App> { |ctx| ... }`, reading state by navigating from `ctx`, not via `&self`. The Finder is the first concrete `TextInput(_)` user — its handler is migrated from binary-side `handle_finder_key` into a free fn referenced from the `Mode::TextInput(...)` variant. The `action_enum!` 3-positional form was locked in Phase 5 and is exercised by `tests/macro_use.rs`; Phase 14's migration is per-call-site only. `ProjectListAction::ExpandRow`/`CollapseRow` vim-extras override goes through `Shortcuts::vim_extras() -> &'static [(Self::Actions, KeyBind)]`.
- **Phase 23 (vim test)** adds a row-rendering check: `VimMode::Enabled` → `ProjectListAction::ExpandRow`'s bar shows `→/l`, `CollapseRow`'s shows `←/h`.

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

**Phase 7 → Phase 8 contract.** The Phase 7 trait surface is **deliberately broken** in this phase. There are no binary call sites yet (the binary swap is Phase 14), so the only consumers are the tui_pane crate's own `cfg(test)` modules and `tests/macro_use.rs` — both rewritten in this phase. Tests written against the Phase 7 surface (e.g. `FooPane::input_mode()`, `pane.label(...)`) are explicitly replaced, not preserved.

**Root re-exports (per Phase 5+ standing rule 2):** crate root gains `pub use pane::{Pane, Mode};` and `pub use bar::Visibility;`; loses `pub use bar::InputMode;`. The Definition of Done at the end of this doc lists the full post-refactor public surface.

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
- Phase 14 trait-tutorial walkthroughs (`### Pane<Ctx>` etc.) need a 4-column table per `feedback_trait_method_table.md`. The current trait surface is small enough that one table per trait is the right granularity.
- The "out of scope (lands in Phase 9)" callout in this phase named `Framework<Ctx>::mode_queries`, `register_app_pane`, and `focused_pane_mode`. **Correction (post-review):** framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) do not go through `register_app_pane` — they lack `AppPaneId`, are constructed inline by `Framework::new()`, and the `mode_queries` registry is for app panes only. `pub(super)` visibility on `register_app_pane` is correct (Phase 9 / 10 / 11 all already say `pub(super)`); the registry writer is internal to the keymap/framework module pair. No `pub(crate)` widening needed.

### Phase 8 Review

- **Phase 9** (Keymap container): renamed stale `Globals::Variant` / `G::Variant` to `Globals::Actions` / `G::Actions` in the dual-source `[global]` merge note (finding 1).
- **Phase 9** (Keymap container): added an explicit "registry constraint" paragraph stating `Framework<Ctx>::register_app_pane` takes `P: Pane<Ctx>` (not `Shortcuts<Ctx>`) so non-shortcut consumers can register (finding 2).
- **Phase 9** (Keymap container): added a verify-step on the `KeyParseError → KeymapError` `#[from]` chain — confirm the variant exists in the shipped Phase 4 enum or add it as a Phase 9 deliverable (finding 5).
- **Phase 11** (Framework panes): rewrote the Toasts paragraph to point at the unified `defaults() / handle_key() / mode() / bar_slots()` inherent surface instead of the inconsistent `dispatch(action, ctx)` mention (finding 7).
- **Phase 11** (Framework panes): noted that `editor_target_path`, `focused_pane_mode`, and `dismiss` are Phase-11 additions on `Framework<Ctx>` (finding 8).
- **Phase 14** (App action enums + impls): added a "no per-impl `#[must_use]`" callout on the `mode()` override snippet — the trait declaration carries it, override bodies inherit (finding 10).
- **Phase 23** (Regression tests): clarified that snapshot tests parameterize on `FocusedPane` and drive through `focused_pane_mode` + `Keymap::render_app_pane_bar_slots`, not a typed `P::mode()` call (finding 12).
- **Phase 8 retrospective** (correction): retracted the "Phase 9 should publish `register_app_pane` as `pub(crate)`" implication — framework panes don't go through registration (they lack `AppPaneId`), so `pub(super)` is correct (finding 3, approved).
- **Trait-associated-type rename:** `P::Action` / `N::Action` / `G::Action` → `P::Actions` / `N::Actions` / `G::Actions` in `scope_for` / `navigation` / `globals` lookups (finding 6, approved & applied).
- **Phase 9** (Keymap container, ErasedScope redesign): replaced the unworkable `action_for(&KeyBind) -> Option<&dyn Action>` / `display_keys_for(&dyn Action) -> &[KeyBind]` surface with three operation-level methods — `dispatch_key(&KeyBind, &mut Ctx) -> KeyOutcome`, `render_bar_slots(&Ctx) -> Vec<RenderedSlot>`, `key_for_toml_key(&str) -> Option<KeyBind>`. Typed access is captured inside the impl block (`ConcreteScope<Ctx, P>`) at registration time; the trait surface stays type-parameter-free. Phase 9 also gains a `RenderedSlot` struct (region/label/key/state/visibility) and re-uses `KeyOutcome` from Phase 11 (finding 4, approved & applied).

### Phase 9 — Keymap container ✅

> **Post-reset note (read first):** The Phase 9 review's Find 2 (`scope_for_typed`) and Find 17 (deferred collapse via `PendingEntry`) were reverted by the **Phase 9 reset** below. The Phase 9 surface that ships in the codebase today is: `pub(crate) RuntimeScope<Ctx>` (renamed from `ErasedScope`), `pub(super) PaneScope<Ctx, P>` (renamed from `ConcreteScope`), three concrete public methods on `Keymap` (`dispatch_app_pane` / `render_app_pane_bar_slots` / `key_for_toml_key`), `AppPaneId`-keyed storage only, typestate `KeymapBuilder<Ctx, State>`. The text below describes the original Phase 9 design as it shipped *before* the reset; jump to the Phase 9 reset subsection for the current state.

Add `Keymap<Ctx>` in `tui_pane/src/keymap/mod.rs` (the keymap module's anchor type lives in its `mod.rs` file, mirroring the Phase 6 precedent of `Framework<Ctx>` in `framework/mod.rs`). `Keymap<Ctx>` exposes `scope_for` / `scope_for_app_pane` / `navigation` / `globals` / `framework_globals` / `config_path` (per §6). Fill in the actual TOML-parsing implementation in `keymap/load.rs` (skeleton + `KeymapError` from Phase 4). Construction is via the canonical entry point `Keymap::<Ctx>::builder()` — an inherent associated function on `Keymap<Ctx>` that returns `KeymapBuilder<Ctx>` — no positionals (the framework owns `GlobalAction` dispatch; see Phase 3 review for full rationale). The builder body itself lands in Phase 10.

**Three scope lookups, one for each consumer.**
- `scope_for::<P>() -> Option<&dyn ErasedScope<Ctx>>` is `TypeId<P>`-keyed and erased; used by code that already has the type parameter and wants to dispatch / render / TOML-lookup through the trait surface.
- `scope_for_typed::<P>() -> Option<&ScopeMap<P::Actions>>` is `TypeId<P>`-keyed and **typed**; used by Phase 15/17 callers that want to test whether a key resolves to a specific action without firing the dispatcher (e.g. `scope_for_typed::<FinderPane>().and_then(|s| s.action_for(&bind)) == Some(FinderAction::Confirm)`). Implementation: `ErasedScope` carries `as_any(&self) -> &dyn Any`; `scope_for_typed` downcasts the trait object to `ConcreteScope<Ctx, P>` and returns `&self.bindings`. **Lands as a Phase 9 amendment at the start of Phase 10.**
- `scope_for_app_pane(id: Ctx::AppPaneId) -> Option<&dyn ErasedScope<Ctx>>` is `AppPaneId`-keyed and used by the bar renderer (Phase 13) and the input dispatcher, both of which hold a `FocusedPane` and never a typed `P`. The `AppPaneId` index is populated at `register::<P>()` time on `P::APP_PANE_ID`. (Framework panes are not in this map; they are special-cased by `FocusedPane::Framework` arms in callers — see Phase 11.)

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
- The "ErasedScope is internal scaffolding" framing in the plan implied `pub(crate)` was the correct visibility. In practice, returning a trait object from a `pub fn` makes any visibility narrower than the trait itself unworkable — every consumer (Phase 11 dispatcher, Phase 13 bar renderer) must name the trait. Sealing is the actual privacy lever, not visibility.
- The plan's `key: ...unwrap_or_default()` line silently assumed `Default for KeyBind`. The cleaner fix (filter unbound slots) collapses two render-time skip paths into one and removes a meaningless default value.
- `Box<dyn Trait>` storage requires `'static` on every type parameter that appears in the trait, not just the trait's own super-bound. `AppContext` itself does not require `'static`, so the bound has to be added at the storage site.

**Implications for remaining phases:**
- Phase 10 builder body inherits the `Ctx: AppContext + 'static` bound — wire it consistently across `with_settings`, `on_quit`, `on_restart`, `dismiss_fallback`, `vim_mode`. The skeleton already has it.
- Phase 11 input dispatcher reaches `dispatch_key` through `Keymap::scope_for_app_pane(id)?.dispatch_key(...)`. The `KeyOutcome::Unhandled` variant is the chain-continue signal. Framework panes use the same enum on inherent methods.
- Phase 13 bar renderer iterates `Keymap::scope_for_app_pane(id)?.render_bar_slots(ctx)` and consumes `RenderedSlot` directly — the typed `Action` enum never crosses the trait, so the renderer is generic over no `<A>` parameter.
- Phase 10 should NOT introduce a `register_navigation` / `register_globals` that returns trait objects — those scopes are singletons (one impl per app), so direct typed storage by `TypeId<N>` / `TypeId<G>` matches the existing core-api §6 design and avoids paying the erasure tax twice.
- **Phase 9 reset shipped the post-reset surface:** typestate builder (`Configuring → Registering`), `pub(crate) RuntimeScope`, three concrete public methods on `Keymap` (`dispatch_app_pane`, `render_app_pane_bar_slots`, `key_for_toml_key`). `Ctx: AppContext + 'static` bound. `BuilderError` dropped.

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

Architect review of remaining phases against the post-reset surface produced 8 findings. 7 applied to the plan text. 1 (Phase 19 cleanup list) was a confirmation pass — no edit needed.

- **Phase 10 doc-sync prerequisite already shipped** (Find 1, applied): obsolete bullet in Phase 9 Review block now strikethrough'd, marked "shipped with the Phase 9 reset."
- **Phase 18 Esc-on-output uses reverse-lookup, not a typed probe** (Find 2, applied): rewrote the snippet to call the public `(AppPaneId, toml_key, KeyBind)` reverse-lookup predicate (`is_key_bound_to_toml_key`) for `OutputAction::Cancel`. No `<P>`-typed probe was re-introduced.
- **`with_navigation` / `with_globals` → `register_navigation` / `register_globals`** (Find 3, applied): startup example at line 381 updated, module-tree comment at line 57 updated.
- **Phase 19 cleanup list** (Find 4, no edit): confirmed the list names no reset-removed types. `PaneScope::new` aside is the only spot mentioning a renamed type, and that's already accurate.
- **Phase 14 overlay walks the binary's known pane set** (Find 5, applied): documented that the binary supplies `(P::APP_PANE_ID, P::Actions::ALL)` pairs to the overlay; no `Keymap::registered_app_panes()` getter required.
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
- The post-reset `Keymap` / `KeymapBuilder` surface was settled by the Phase 9 reset commit, so Phase 10 starts without a doc-sync prerequisite.
- A test was added at the `Keymap` boundary (`render_app_pane_bar_slots_resolves_through_keymap`) the original plan didn't call out — it was a gap once `scope_for_app_pane` went away.

**Surprises:**
- `unnecessary_wraps` clippy lint fired on `finalize` — Phase 9's plan had `build()` return `Result` for forward compatibility with Phase 10 errors, but the helper fn that does the work doesn't need to wrap. Solution: helper returns `Keymap<Ctx>`, `build` wraps in `Ok(...)`. Phase 10 will tighten this when real errors land.
- The Phase 9 retrospective and Phase 9 Review blocks both documented the to-be-reverted amendments (`scope_for_typed`, `PendingEntry`) — those entries now read as "shipped → reverted." Annotated with `~~strikethrough~~` rather than deleted, because the reasoning behind the original choice is still useful context.

**Implications for remaining phases:**
- **Phase 10:** every settings method (`load_toml` / `vim_mode` / `with_settings` / `register_navigation` / `register_globals` / `on_quit` / `on_restart` / `dismiss_fallback`) lives on the `Configuring`-state impl. `register::<P>` is the one method that exists on both states, with identical bodies — Phase 10's typed work (TOML overlay, vim extras) runs inside that body, not in a deferred `build()` walk.
- **Phase 11:** dispatcher calls `keymap.dispatch_app_pane(id, &bind, ctx)`, not the (now-gone) `scope_for_app_pane(id)?.dispatch_key(...)` chain. `KeyOutcome::Unhandled` semantics unchanged.
- **Phase 13:** bar renderer calls `keymap.render_app_pane_bar_slots(id, ctx)` for the `App(id)` arm. Framework-pane arm unchanged.
- **Phase 15/17:** can no longer use `scope_for_typed::<P>().and_then(...)` to probe an action without dispatching. Two options: (a) dispatch through `dispatch_app_pane` and observe a side effect (atomic counter, captured value), (b) add a `cfg(test) pub(crate)` typed-action probe at the phase that needs it. The plan now points to (a) by default; (b) lands per-phase if the test really needs it.
- **Phase 20:** keymap overlay reads typed singletons (`Keymap::navigation::<N>` / `Keymap::globals::<G>`) — those are typed by design, unaffected by the reset.

### Phase 9 Review

Architect review of remaining phases against shipped Phase 9 produced 18 findings. 11 minor were applied silently to the plan text. 7 significant were reviewed with the user; outcomes below.

**Phase 9 amendments to land at the start of Phase 10** (since Phase 9 already shipped):
- ~~**Add typed scope accessor** (Find 2): `Keymap::scope_for_typed::<P>()`.~~ **Reverted by Phase 9 reset.** Test/inspection access doesn't belong on the public erased trait. Tests go through `dispatch_app_pane` and observe side effects.
- ~~**Defer `into_scope_map()` collapse to `build()`** (Find 17).~~ **Reverted by Phase 9 reset.** Eager collapse in `register::<P>` works once the typestate enforces "settings before panes." Phase 10's TOML overlay and vim extras land inline in `register::<P>` instead of via deferred collapse.

**Phase 10 plan changes:**
- **One error type, not two** (Find 12): `KeymapError` gains three variants (`NavigationMissing`, `GlobalsMissing`, `DuplicateScope { type_name: &'static str }`) and remains the sole failure type. `BuilderError` is dropped from the plan and from core-api §7. `KeymapBuilder::build()`'s signature stays `Result<Keymap<Ctx>, KeymapError>` — Phase 9 tests do not change.
- **Typed singleton storage for `Navigation` / `Globals`** (Find 13): `Keymap<Ctx>` adds `navigation: Box<dyn Any + Send + Sync>` and `globals: Box<dyn Any + Send + Sync>` fields, populated by `KeymapBuilder::register_navigation::<N>()` and `register_globals::<G>()`. Public getters: `pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Actions>` and `pub fn globals<G: Globals<Ctx>>(&self) -> &ScopeMap<G::Actions>`. Pane scopes stay erased (heterogeneity is the reason); singletons stay typed (Phase 23's `key_for(NavigationAction::Up)` reads through the public getter, no downcast at the call site). `framework_globals: ScopeMap<GlobalAction>` already typed — unchanged.
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
  GlobalAction::Dismiss → dismiss_chain(ctx, fallback) (Phase 12 free fn)
    → framework_mut().dismiss_framework()
        → toasts.dismiss_focused() when focused on Toasts
        → close_overlay() otherwise
    → dismiss_fallback hook (binary's optional opt-in)

Toasts::handle_key is a stub returning Unhandled. Visible-but-not-
focused toasts ignore key input by virtue of the routing — no
pane-local check needed.
```

**Phase 11/15/17 snippet rewrites (Find 1, post-reset):** every plan snippet of the form `keymap.scope_for::<P>().action_for(&bind) == Some(SomeAction)` is replaced by either (a) dispatching through `keymap.dispatch_app_pane(P::APP_PANE_ID, &bind, ctx)` and observing the dispatcher's side effect, or (b) a `cfg(test) pub(crate)` test-only typed-action probe added in the affected phase. The Phase 9 reset dropped `scope_for_typed`; Phase 15 (Finder Confirm/Cancel) and Phase 18 (Esc-on-output) take the dispatch-and-observe form. Phase 20's keymap overlay reads typed singletons (`Keymap::navigation::<N>` / `Keymap::globals::<G>`) and the typed-public-method form is unchanged from Phase 10's plan.

**Phase 14 plan changes (Find 6):**
- The bar renderer matches `FocusedPane` first. Only the `App(id)` arm calls `Keymap::render_app_pane_bar_slots(id, ctx)` and consumes `RenderedSlot`. The `Framework(fw_id)` arm calls each framework pane's inherent `bar_slots()` method directly — the keymap is never queried for framework-pane bar contents.
- Region modules (`nav_region`, `pane_action_region`, `global_region`) filter the flat `Vec<RenderedSlot>` by `region` field — they no longer walk typed `BarSlot<A>` payloads. Replace plan wording that names tuple patterns like `(Nav, _)` with field-match on `RenderedSlot { region: BarRegion::Nav, .. }`.

**Phase 14 binary plan changes (Find 7, Find 8 minor):**
- `Keymap` overlay drives off `P::Actions::ALL` (from the `Action` trait), then calls `keymap.key_for_toml_key(P::APP_PANE_ID, action.toml_key())` per action to fetch the bound key. Unbound actions render with an empty key cell so the user can rebind them — `render_bar_slots` (which drops unbound) is the wrong API for the overlay; that one is the **status bar's** API. The overlay walks the registered pane set by iterating the binary's known list of `(P::APP_PANE_ID, P::Actions::ALL)` pairs — the binary already knows its panes, so no `Keymap::registered_app_panes()` getter is required.
- `key_for_toml_key` returning `None` for "unknown action" vs "known action, no binding" is treated identically by the overlay (both render as unbound). The trait method does not need to distinguish them.

**Phase 19 plan changes (Find 14 minor):** Phase 19's `const fn` deletion list applies only to the pre-refactor binary types (`Shortcut::from_keymap` / `disabled_from_keymap`). New `tui_pane` `const fn`s (`BarSlot::primary`, framework `const fn` getters) are kept — do not run a careless `s/const fn/fn/` sweep. (`PaneScope` no longer has a `new` constructor post-reset — fields are `pub(super)` and the builder constructs with a struct literal.)

**Phase 23 plan changes (Finds 9, 15 minor):**
- Dispatch parity tests assert via the dispatcher's side effect (atomic counter, captured value), not the return. `KeyOutcome::Consumed` only tells the caller a binding fired; *which* action fired is observed through the dispatcher.
- Add a `KeymapError::KeyParse` propagation regression test: round-trip a malformed binding string through the loader, assert the variant matches and the source is preserved.

**Findings rejected:** none. All seven significant findings produced plan changes; the 11 minor findings either applied directly (where actionable) or confirmed existing plan text (no change needed).


### Phase 10 — Keymap builder + settings registry ✅

**Phase 9 reset already shipped:** the builder skeleton has the typestate (`Configuring` → `Registering`), `register::<P>` does eager collapse, the public surface is three concrete methods on `Keymap` (`dispatch_app_pane` / `render_app_pane_bar_slots` / `key_for_toml_key`), and the trait is `pub(crate) RuntimeScope<Ctx>`. Phase 10 adds the settings phase methods and the framework integration that hook onto that scaffolding.

Two tightly-coupled additions in one commit because `KeymapBuilder::with_settings` is the only consumer of `SettingsRegistry`:

- `tui_pane/src/settings.rs` — `SettingsRegistry` + `add_bool` / `add_enum` / `add_int` / `with_bounds` (§9).
- `tui_pane/src/keymap/builder.rs` — `KeymapBuilder<Ctx, Configuring>` body fills in. One error type — `KeymapError` — covers loader and builder validation; three new variants land here: `NavigationMissing`, `GlobalsMissing`, `DuplicateScope { type_name: &'static str }`. `BuilderError` was rejected during Phase 9 review (one type beats two when the binary's startup path renders both the same).

**Typed singleton storage for `Navigation` / `Globals`.** `Keymap<Ctx>` gains three fields populated by `KeymapBuilder::register_navigation::<N>()` and `register_globals::<G>()` (both on the `Configuring` state):

```rust
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:            HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>, // pane scopes (erased — heterogeneous)
    navigation:        Option<Box<dyn Any>>,                                // Some(ScopeMap<N::Actions>) post-build
    globals:           Option<Box<dyn Any>>,                                // Some(ScopeMap<G::Actions>) post-build
    framework_globals: ScopeMap<GlobalAction>,                              // already typed (no <Ctx>)
    config_path:       Option<PathBuf>,
}

pub fn navigation<N: Navigation<Ctx>>(&self) -> Option<&ScopeMap<N::Actions>> { /* downcast */ }
pub fn globals<G: Globals<Ctx>>(&self)       -> Option<&ScopeMap<G::Actions>> { /* downcast */ }
```

Pane scopes stay erased (heterogeneity is the reason). Singletons stay typed — Phase 23's `key_for(NavigationAction::Up)` reads through the public getter without a downcast at the call site (the getter does it). Framework globals stay typed inline.

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
- `OpenKeymap` / `OpenSettings` → `ctx.framework_mut().open_overlay(FrameworkPaneId::Keymap | Settings)`. The overlay layer is orthogonal to focus — `focused` is left untouched. (Closure-of-Phase-10 amendment; the originally-shipped `set_focus(...)` form was switched to `open_overlay(...)` to match the binary's existing modal-layer model.)
- `Dismiss` → calls `ctx.framework_mut().close_overlay()`; if it returns `true`, the dispatcher returns. Otherwise it falls through to the optional `dismiss_fallback` hook. Phase 11 inserts the toasts arm in front of the overlay-clear step (full chain: focused-toasts pop → overlay close → `dismiss_fallback`).

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

### Retrospective

**What worked:**
- Typestate transition `Configuring → Registering` enforces "settings before panes" at compile time — the `compile_fail` doctest on `KeymapBuilder` locks the contract and survived clippy / fmt without changes.
- `Box<dyn Any>` typed-singleton storage with `downcast_ref::<ScopeMap<X::Actions>>` keeps `Keymap<Ctx>` heterogeneous across `<N::Actions>` / `<G::Actions>` without forcing `Send + Sync` bounds on every action enum.
- Eager TOML overlay inline at `register::<P>` (per the Phase 9 reset) was straightforward to write — no `PendingEntry` deferred-collapse state to thread through. Vim extras append in the same spot, after TOML.
- Deferred-error capture (`deferred_error: Option<KeymapError>` on the builder) lets `register::<P>` keep its `Self`-returning chain signature while still surfacing per-pane overlay failures from `build` / `build_into`.
- TOML loader surface — `load_toml(PathBuf) -> Result<Self, KeymapError>` with `NotFound` treated as "no overlay" — round-tripped cleanly through every test path on the first attempt.

**What deviated from the plan:**
- `Keymap::navigation::<N>()` / `globals::<G>()` return `Option<&ScopeMap<...>>` instead of `&ScopeMap<...>`. Reason: `clippy::expect_used` is in the workspace lint config and the panic path needed a per-method `#[allow]` to satisfy it. Returning `Option` matches the underlying storage state and shifts the unwrap to call sites (Phase 11 dispatcher, binary code).
- `register_navigation::<N>()` / `register_globals::<G>()` take no value parameter (plan: `register_navigation(N)`). Reason: `clippy::needless_pass_by_value` rejected an unused `_navigation: N` arg, and `N` / `G` are typically ZST markers — the value carries no data.
- `Framework::new` is no longer `const fn`. The Phase 6 contract said the five frozen methods stay `const`, but `HashMap::new()` is not `const fn` in stable Rust; adding the `mode_queries` / `pane_order` fields broke const-eligibility for `new`. The other four frozen methods (`focused`, `set_focused`, `quit_requested`, `restart_requested`) stayed `const`.
- `on_quit` / `on_restart` / `dismiss_fallback` hooks live on `Keymap<Ctx>`, not on `Framework<Ctx>`. The plan said the dispatcher "closes over" them, but a free `fn` cannot close over anything; routing through `&Keymap<Ctx>` is the actual mechanism.
- `dispatch_global` signature is `(action, &Keymap<Ctx>, &mut Ctx)` not `(action, &mut Ctx)` — same reason as above. `Keymap::dispatch_framework_global(action, ctx)` is the public call-site entry point that wraps the free fn.
- Cross-action collision validation uses a new `pub(super) Bindings::entries()` accessor + a small `bindings_entries` helper in `builder.rs`. The plan didn't call this out; it fell out of needing to walk a `Bindings` after overlay without re-collapsing into a `ScopeMap` first.

**Surprises:**
- `Box<dyn Any + Send + Sync>` (the plan's storage type for typed singletons) refused to compile against generic `<N::Actions>` because the `Action` trait does not bound `Send + Sync`. Dropped to `Box<dyn Any>` — this is single-threaded UI code; nothing on the call path actually requires the bounds.
- `Keymap` cannot `derive(Debug)` because `Box<dyn Any>` is not `Debug`. Manual `impl Debug` with `finish_non_exhaustive` was needed for the test suite (`expect_err` requires `Debug` on the `Ok` payload).
- `clippy::expect_used` and `clippy::needless_pass_by_value` together drove three small API changes (typed-singleton getters return `Option`, `register_navigation` / `register_globals` drop their value parameter, `load_toml` consumes its `PathBuf` directly). None changed the user-facing contract — production callers still get the same chain.
- TOML overlay's "replace-per-action" semantics required adding `Bindings::override_action(&A, Vec<KeyBind>)` (drops existing entries for the action, pushes the new ones). No public surface impact; `pub(super)` keeps it loader-internal.

**Implications for remaining phases:**
- Phase 11 input dispatcher: route `GlobalAction` hits through `keymap.dispatch_framework_global(action, ctx)`, not the free fn directly. The dispatcher chain in Phase 11's plan body still names the free fn — sync the doc to the public method.
- Phase 11 Navigation / Globals dispatch sites must unwrap `keymap.navigation::<N>()` / `keymap.globals::<G>()` (e.g. with `.expect("registered")` or a `let Some(_) = ... else { return; }`). Production callers can rely on `Some(_)` because `KeymapError::NavigationMissing` / `GlobalsMissing` block any build with registered panes, but the type now demands the unwrap.
- Phase 11 dismiss chain: extend the Phase-10-closure `Dismiss` arm (currently `close_overlay()` → `dismiss_fallback`) to the full chain (`Framework::dismiss()` covering focused-toasts pop → overlay close → return false → `dismiss_fallback`). The toasts pop is the only piece Phase 11 still needs to add; overlay close already shipped.
- Phase 11 / docs: `Framework::new` is plain `fn`, not `const fn`. Anything in the remaining-phase plan that treats `Framework::new(initial_focus)` as const-evaluable is now wrong — sync.
- Phase 11 `Framework` access: `pane_order()` is `pub(super)` (Phase 10 added it for `dispatch_global`'s `NextPane` / `PrevPane`). If Phase 11's bar renderer or input dispatcher needs the order, it has to widen visibility or call through the framework's existing methods.
- Test gaps documented in `missing_tests.md` at repo root: items 1 (vim full-`KeyBind` equality), 2 (cross-action collision via TOML), 5 (`with_settings` round-trip), 7 (`[global]` TOML overlay onto framework globals), 8 (`NextPane` / `PrevPane` dispatch), 9 (`OpenKeymap` / `OpenSettings` dispatch) belong in Phase 10's closure or fold into Phase 11's first commit; items 3 (`InvalidBinding`), 4 (`UnknownAction`), 10 (`DuplicateScope` `type_name` payload assertion) are nice-to-have.
- `bindings.rs` now exposes `pub(super) override_action` / `has_key` / `entries()`. Phase 15+ retrospective tests can read `entries()` directly instead of round-tripping through `ScopeMap`.

### Phase 10 Review

- **Phase 10 body (lines 1797–1798):** synced typed-singleton storage type from `Box<dyn Any + Send + Sync>` → `Box<dyn Any>` and getter return from `&ScopeMap<...>` → `Option<&ScopeMap<...>>` to match shipped code.
- **Phase 11 `focused_pane_mode` block:** updated return from `Mode<Ctx>` → `Option<Mode<Ctx>>` and added a paragraph noting Phase 10 shipped the `Option` form; Phase 11 fills in framework-pane arms.
- **Phase 11 dispatch chain:** rewrote arms (a)/(b)/(c) to name `keymap.dispatch_framework_global(action, ctx)` and to use `if let Some(scope) = keymap.{globals,navigation}::<_>()` patterns matching the shipped getter signatures.
- **Phase 11 Dismiss arm:** added the explicit instruction to modify `framework/dispatch.rs`'s Phase-10 Dismiss arm to call `framework_mut().dismiss()` first, falling through to `dismiss_fallback` only on `false`.
- **Phase 11 framework-pane wording:** tightened — framework panes lack `APP_PANE_ID` because `Pane<Ctx>` (the supertrait of `Shortcuts<Ctx>`) declares it.
- **Phase 11 `pane_order()` visibility:** added a Phase-11 step to widen from `pub(super)` to `pub` for Phase 13 / 19 callers.
- **Phase 13 region modules:** updated all `matches!(focused_pane_mode(ctx), Mode::X)` predicates to `Some(Mode::X)`; spelled out the framework-pane bar adapter (walks `bar_slots()` + `Bindings::entries()` from inherent `defaults()`; widen `Bindings::entries` from `pub(super)` to `pub(crate)` in Phase 13).
- **Phase 14 tests:** added `build_into` preflight requirement — tests that exercise `framework.focused_pane_mode(ctx)` must build the keymap with `build_into(&mut framework)`, not `build()`.
- **Phase 18 structural Esc snippet:** updated `focused_pane_mode` match to `Some(Mode::TextInput(_))`.
- **Phase 19 deletion list:** added the rule that any pre-existing pre-quit / pre-restart cleanup paths in the binary move into `.on_quit` / `.on_restart` closures registered on the keymap builder.
- **Phase 11 — significant, approved & integrated:** Phase 11 body re-architects overlays as a separate `overlay: Option<FrameworkPaneId>` modal layer (matches binary's existing model) instead of moving `framework.focused`. Drops the need for any `previous_focus` field. Affected sections: Phase 11 focus-model intro, the dispatch chain code block (overlay-first), `Framework::dismiss()` body, `focused_pane_mode` (consults overlay first), `editor_target_path` (consults overlay first), `Framework<Ctx>` fields (adds `overlay`), methods (adds `overlay()` getter + `pub(super) open_overlay`), Phase 13 bar renderer top-level dispatch, Phase 10 dispatcher table footnote.
- **Phase 11 — significant, approved & integrated:** Toasts model integrated — `FocusedPane::Framework(Toasts)` stays a real focus state (Tab-focusable when `toasts.has_active()`); `dismiss()` calls `try_pop_top()` only when focused on Toasts; no auto-focus when a toast appears.

#### Phase 10 closure (overlay scaffolding pulled forward from Phase 11)

Done at the request to clear the deck before Phase 11. Two flagged items, both shipped:

1. **Overlay field + accessors on `Framework<Ctx>`.** Added `overlay: Option<FrameworkPaneId>` with `pub const fn overlay()` getter, `pub(super) const fn open_overlay(FrameworkPaneId)` setter, and `pub(super) const fn close_overlay() -> bool` setter. Phase 11's full `Framework::dismiss()` chain reuses `close_overlay()` as its middle arm.
2. **Dispatcher rewrite.** `framework/dispatch.rs` `OpenKeymap` / `OpenSettings` now call `framework_mut().open_overlay(...)` instead of `set_focus(FocusedPane::Framework(...))`. `Dismiss` calls `framework_mut().close_overlay()` first; if it returns `true`, the dispatcher returns; otherwise it falls through to `dismiss_fallback`. The orthogonal-overlay model now matches the binary's existing modal-layer behavior 1:1.
3. **Test rewrite.** `tui_pane/src/keymap/builder.rs` test renamed `open_keymap_and_open_settings_focus_framework_overlays` → `open_keymap_and_open_settings_open_framework_overlays`; now asserts `framework.overlay() == Some(...)` and that `framework.focused()` does not move during open. Extended to also assert `Dismiss` clears the overlay.
4. **Variant decision (resolved).** Kept `FrameworkPaneId` unified (`Keymap | Settings | Toasts`); the `FocusedPane::Framework(Keymap | Settings)` focus arms are unreachable post-overlay-switch but remain valid payloads. Phase 11 match sites mark those arms with `// unreachable post-overlay-switch` comments rather than panicking. Recorded under Phase 11 focus-model intro.

`cargo build -p tui_pane` clean, `cargo nextest run -p tui_pane` 142/142 pass, `cargo clippy -p tui_pane --all-targets --all-features -- -D warnings` clean.

### Phase 11 — Framework panes ✅

Phase 11 fills in the framework panes inside the **existing** `Framework<Ctx>` skeleton from Phase 6. The struct's pane fields and helper methods land here; the type itself, `AppContext`, and `FocusedPane` already exist.

**Focus model — overlays are orthogonal to focus, matching the binary 1:1.** Audit of `src/tui/overlays/mod.rs` and `src/tui/input.rs:126-137` confirmed: today's binary keeps `app.focus` on the underlying pane while Settings/Keymap/Finder open/close as separate modal-mode state. Only `PaneId::Toasts` is ever directly focused (via Tab). Phase 11 mirrors that:

- `Framework<Ctx>` carries an `overlay: Option<FrameworkPaneId>` field, separate from `focused`. `None` = no overlay, `Some(Keymap)` / `Some(Settings)` = the overlay is open over the underlying focused pane. *(Shipped at Phase 10 closure: field + `pub const fn overlay()` getter + `pub(super) const fn open_overlay(FrameworkPaneId)` setter + `pub(super) const fn close_overlay() -> bool` setter.)*
- `OpenKeymap` / `OpenSettings` write `overlay`, never `focused`. *(Shipped at Phase 10 closure: `framework/dispatch.rs` calls `ctx.framework_mut().open_overlay(...)` directly.)* The Phase 10 dispatcher also already wires the `Dismiss` arm to call `close_overlay()` and fall through to `dismiss_fallback` only when no overlay was open.
- `Framework::dismiss()` (Phase 11) becomes the full chain: focused-toasts pop → `close_overlay()` → return `false`. Phase 11 reuses the existing `close_overlay()` method as the middle arm. No `previous_focus` field, no prior-focus tracking — focus never moves.
- `focused_pane_mode`, the bar renderer (Phase 13), and the dispatch chain consult `overlay` first; fall through to `focused` when `None`.
- `set_focused` stays a frozen Phase 6 `const fn` setter — no overlay-aware branching, no contract change.
- `FocusedPane::Framework(Toasts)` stays valid as a Tab-focusable state (matches binary's `PaneId::Toasts` Tab behavior). `FocusedPane::Framework(Keymap | Settings)` is unreachable by construction (the dispatcher never writes those focus states post-Phase-10 closure). *(Shipped at Phase 10 closure: the test `open_keymap_and_open_settings_open_framework_overlays` at `tui_pane/src/keymap/builder.rs` asserts against `framework.overlay()` and confirms `framework.focused()` does not move.)*

**Toasts focus model — `Toasts<Ctx>` is a placeholder pane with a message stack.** Phase 11 ships `Toasts<Ctx>` as a minimal typed pane that owns a `Vec<String>` message stack with `push`/`try_pop_top`/`has_active`. A single `ToastsAction::Dismiss` action pops the top toast. `Mode::Static` (no scrolling, no text input). `bar_slots` returns `[(PaneAction, Single(Dismiss))]`. The pane is held inline on `Framework<Ctx>` as `pub toasts: Toasts<Ctx>`, the same field-wise treatment as `keymap_pane` / `settings_pane`.

The framework's `Tab`/`Shift+Tab` cycle does not include Toasts at this phase — `focus_step` walks `pane_order()` (app panes only) and early-returns on any `FocusedPane::Framework(_)`. Phase 12 takes the next step: replacing this placeholder with a typed `Toast` manager, splitting `FrameworkPaneId` into overlay/focus enums, and rewriting `focus_step` to include Toasts as a virtual cycle entry when `has_active()` returns `true`.

**Hard dependency on Phase 10.** The dispatcher chain below calls `keymap.framework_globals()`, `keymap.globals::<G>()`, and `keymap.navigation::<N>()` — all three are added by Phase 10 (typed singleton getters + the storage they read). Phase 11 cannot land until Phase 10 ships those.

**Mixing const and non-const inside `impl Framework<Ctx>` is intentional.** The five Phase 6 methods (`new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`) stay `const fn` verbatim. The Phase 11 additions (`dismiss`, `editor_target_path`, `focused_pane_mode`, etc.) call into `HashMap` lookups and pane state, neither of which is const-eligible — those land as plain `fn`. Standing rule 9 still applies (every `&self` value-returning method gets `#[must_use]`; const where eligible).

**`Toasts<Ctx>` is held inline, not boxed.** The new `toasts: Toasts<Ctx>` field lives directly on `Framework<Ctx>`. Dispatchers reach it via `ctx.framework().toasts.has_active()` (Phase 11 placeholder) and `ctx.framework_mut().dismiss_framework()` (Phase 12 typed manager). No `Rc`/`RefCell`/`Cell` wrappers — single-threaded ownership through `&mut Ctx` is the contract.

> **Phase 6 → Phase 11 contract (mirror).** Purely additive: this phase adds fields and methods, but the Phase 6 surface — 3 frozen fields (`focused`, `quit_requested`, `restart_requested`) plus 5 frozen method **signatures** (`new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`) — must keep its names and signatures exactly. The `const fn` qualifier is preserved on the four getter/setter methods (`focused`, `set_focused`, `quit_requested`, `restart_requested`); `new`'s body grows new field initializers each phase (Phase 10 added `HashMap::new()` and is therefore no longer `const fn`, Phase 11 adds the three pane defaults), so its qualifier is "frozen-as-shipped-by-Phase-10," not "frozen-as-Phase-6." Tests written in Phases 7–10 against the skeleton must continue to pass at the end of Phase 11. If Phase 11 surfaces a better name or signature for any of the frozen items, that is a deliberate breaking change — surface it as a follow-up, not a silent rename.

Add to `tui_pane/src/panes/`:

- `keymap.rs` — `KeymapPane` with internal `EditState::{Browse, Awaiting, Conflict}`. Method `editor_target(&self) -> Option<&Path>`. `mode(&self, ctx) -> Mode<Ctx>` returns `TextInput(keymap_capture_keys)` when `EditState == Awaiting`, `Static` when `Conflict`, `Navigable` when `Browse`.
- `settings.rs` — `SettingsPane` with internal `EditState::{Browse, Editing}`; uses `SettingsRegistry`. Method `editor_target(&self) -> Option<&Path>`. `mode(&self, ctx) -> Mode<Ctx>` returns `TextInput(settings_edit_keys)` when `EditState == Editing`, `Navigable` otherwise.
- `toasts.rs` — `Toasts<Ctx>` is a placeholder pane carrying a `Vec<String>` message stack. Public surface: `new()`, `push(impl Into<String>)`, `try_pop_top() -> bool`, `has_active() -> bool`, `defaults() -> Bindings<ToastsAction>` (binds `Esc → Dismiss`), `handle_key(&mut self, &mut Ctx, &KeyBind) -> KeyOutcome` (returns `Consumed` on `Dismiss`, `Unhandled` otherwise — the only framework pane whose `handle_key` may return `Unhandled`), `mode(&self, &Ctx) -> Mode<Ctx>` (always `Mode::Static`), `bar_slots(&self, &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)>`. The message stack is the placeholder content store; Phase 12 replaces it with a typed `Toast` manager.

**Inherent action surface — same four methods on all three framework panes.** `KeymapPane`, `SettingsPane`, and `Toasts<Ctx>` each ship:
- `pub fn defaults() -> Bindings<Self::Action>` — same role as `Shortcuts::defaults`, no trait.
- `pub fn handle_key(&mut self, ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome`. The two overlay panes intercept ALL keys when focused and return `KeyOutcome::Consumed` regardless (matches existing cargo-port `keymap_open` / `settings_open` short-circuit behavior). `Toasts::handle_key` returns `Consumed` on `Dismiss`, `Unhandled` otherwise — the only framework pane whose `handle_key` may return `Unhandled`.
- `pub fn mode(&self, ctx: &Ctx) -> Mode<Ctx>` — `&self` form (the framework owns the struct directly, no split-borrow constraint).
- `pub fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Action>)>` — same role as `Shortcuts::bar_slots`.

These mirror the trait method set per-method, but as inherent methods so the `Pane<Ctx>::APP_PANE_ID` constraint doesn't apply. The bar renderer and dispatcher walk `FocusedPane::App(_)` through the trait surface and `FocusedPane::Framework(_)` through these inherent methods.

**Dispatch chain (matches existing cargo-port `src/tui/input.rs::handle_key_event` order).** The framework input dispatcher routes `KeyEvent` through this chain:

```text
Pre-flight (binary-specific structural escapes — keep cargo-port behavior verbatim):
  1. Esc + framework example_running (or app's equivalent) → kill PID, return.
  2. Esc + non-empty output buffer → clear output, refocus, return.
  3. Confirm modal active → consume key (y/n only), return.

Overlay layer first (overlays sit on top of the focused pane):

  if let Some(overlay) = framework.overlay():
    Some(Keymap)   → framework.keymap_pane.handle_key(ctx, &bind)
    Some(Settings) → framework.settings_pane.handle_key(ctx, &bind)
    Both overlays intercept ALL keys when open and return
    KeyOutcome::Consumed regardless (matches existing cargo-port
    `keymap_open` / `settings_open` short-circuit behavior). Return.

Then match focused pane:

  FocusedPane::Framework(Toasts):
    framework.toasts.handle_key(ctx, &bind)
    Returns Consumed on Dismiss (pops the top toast); Unhandled
    otherwise. Falls through to step (a) below on Unhandled — globals
    and dismiss still fire from any pane.

  FocusedPane::App(id) (or Framework(Toasts) → Unhandled fall-through):
    a. Framework globals first: keymap.framework_globals().action_for(&bind)
       → if Some(action), call keymap.dispatch_framework_global(action, ctx)
       (the public wrapper around the pub(crate) free fn dispatch_global,
       which closes over the keymap's hook fn pointers). Handles
       Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss.
       Returns Consumed on hit.
    b. App globals next: if let Some(scope) = keymap.globals::<G>() then
       scope.action_for(&bind) → if Some, G::dispatcher() runs. Returns
       Consumed on hit. The Some(scope) branch is the production path —
       KeymapError::GlobalsMissing already blocks any build with
       registered panes but no globals registered, so production code can
       rely on Some, while test code that builds without globals takes
       the None branch as a no-op. (The shared [global] TOML table merges
       both sources at load time — Phase 9 loader-decisions.)
    c. Navigation scope: same Option-handling pattern —
       if let Some(scope) = keymap.navigation::<N>() { scope.action_for(&bind) }
       — if Some(action), N::dispatcher() routes by FocusedPane to the
       focused scrollable surface. Returns Consumed on hit. Same
       missing-singleton invariant as (b): KeymapError::NavigationMissing
       blocks production builds.
    d. Per-pane scope: keymap.dispatch_app_pane(id, &bind, ctx).
       Returns Consumed or Unhandled (Unhandled if no scope is
       registered for `id` or no binding matches).
    e. Unhandled → drop the key (no further fallback).

Dismiss is the named global action, not an Unhandled fallback:
  GlobalAction::Dismiss → if framework.dismiss() returns true, stop;
  otherwise call the binary's optional `dismiss_fallback` hook.
  Order inside `framework.dismiss(&mut self) -> bool`:
    1. If focused on Toasts and the stack is non-empty → pop the top
       toast; return true.
    2. If an overlay is open → close it; return true.
    3. Otherwise → return false; the dispatcher then calls
       `dismiss_fallback` if registered.
  Fires only when the bound key resolves to Dismiss — never on every
  Unhandled.
```

This is a strict generalization of today's `handle_key_event` order. The `keymap_open` / `settings_open` short-circuits become the overlay-layer arm at the top of dispatch (consulting `framework.overlay()`). The `handle_global_key` step becomes (a)+(b). `handle_normal_key`'s hardcoded nav becomes (c). Per-pane keymap dispatch becomes (d). The cargo-port behavior stays byte-identical under default bindings.

**Extend `tui_pane/src/framework/mod.rs`** — keep the three Phase-6 frozen-signature fields and the four `const fn` getters/setters verbatim (`focused`, `set_focused`, `quit_requested`, `restart_requested`); `new`'s body grows pane-default initializers (the function stays non-`const fn` as of Phase 10, see the mirror block above). Keep the four Phase-10 / Phase-10-closure additions verbatim (`mode_queries`, `pane_order`, `overlay`, plus the four accessor methods); add the new pane fields and the new methods. Do *not* rewrite the struct as a wholesale replacement; this is a strict superset of what Phases 6 / 10 already shipped.

Fields after Phase 11 (Phase 6 frozen fields and Phase-10-shipped fields stay verbatim, in their original positions):

```rust
pub struct Framework<Ctx: AppContext> {
    // ── Phase 6 frozen fields (unchanged) ──
    focused:           FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,

    // ── Phase 10 / Phase-10-closure shipped fields ──
    mode_queries:      HashMap<Ctx::AppPaneId, ModeQuery<Ctx>>,
    pane_order:        Vec<Ctx::AppPaneId>,
    overlay:           Option<FrameworkPaneId>,

    // ── Phase 11 additions ──
    pub keymap_pane:   KeymapPane,
    pub settings_pane: SettingsPane,
    pub toasts:        Toasts<Ctx>,
}
```

Methods after Phase 11 (the five Phase 6 const-fn methods plus the four Phase-10 / Phase-10-closure methods — `register_app_pane`, `pane_order`, `overlay`, `open_overlay`, `close_overlay` — stay verbatim). `editor_target_path`, `focused_pane_mode`, and `dismiss` are Phase-11 additions. Phase 11 also rewrites `focused_pane_mode` to consult `overlay` first (Phase 10 returned `None` for any framework focus state):

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    pub fn editor_target_path(&self) -> Option<&Path> {
        match self.overlay {
            Some(FrameworkPaneId::Keymap)   => self.keymap_pane.editor_target(),
            Some(FrameworkPaneId::Settings) => self.settings_pane.editor_target(),
            Some(FrameworkPaneId::Toasts) | None => None,
        }
    }

    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Option<Mode<Ctx>> {
        // Overlay layer wins over focus when open.
        match self.overlay {
            Some(FrameworkPaneId::Keymap)   => return Some(self.keymap_pane.mode(ctx)),
            Some(FrameworkPaneId::Settings) => return Some(self.settings_pane.mode(ctx)),
            Some(FrameworkPaneId::Toasts) | None => {} // fall through
        }
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Toasts)    => Some(self.toasts.mode(ctx)),
            FocusedPane::Framework(FrameworkPaneId::Keymap
                                  | FrameworkPaneId::Settings) => None, // not reachable post-Phase-11
            FocusedPane::App(app)                                => self.mode_queries.get(&app).map(|q| q(ctx)),
        }
    }

    /// Run the framework dismiss chain. Returns `true` when something
    /// was dismissed at the framework level. The
    /// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss)
    /// dispatcher consults this; on `false`, it falls through to the
    /// binary's optional `dismiss_fallback` hook.
    ///
    /// Order:
    /// 1. If [`Self::focused`] is the toast stack, pop the top toast.
    /// 2. Else if an overlay is open, close it.
    /// 3. Else return `false`.
    pub fn dismiss(&mut self) -> bool {
        if matches!(self.focused, FocusedPane::Framework(FrameworkPaneId::Toasts))
           && self.toasts.try_pop_top()
        {
            return true;
        }
        self.close_overlay()
    }
}
```

**Phase 10 already shipped `focused_pane_mode` with the `Option<Mode<Ctx>>` return type** (returns `None` for any `FocusedPane::Framework(_)` arm because Phase 10 had no framework-pane structs to query). Phase 11 modifies the body to consult `overlay` first; the focus-arm match remains as the second tier. Callers in Phases 12 and 16 must handle `Option` (e.g. `matches!(focused_pane_mode(ctx), Some(Mode::Navigable))`).

Callers pass the same `&App` they hold; the method takes `&Ctx` because the framework is generic, but `Ctx == App` in cargo-port and the `&App` derefs cleanly.

The registry is populated by `KeymapBuilder::build_into(&mut framework)`: for each `P: Pane<Ctx>` registered on the builder, the chain calls `P::mode()` (the trait associated function on `Pane<Ctx>`) to obtain the `fn(&Ctx) -> Mode<Ctx>` pointer and hands it to `framework.register_app_pane(P::APP_PANE_ID, query)`. `register_app_pane` is `pub(super)` so only the builder writes the registry; the field stays private.

`Framework<Ctx>` lives in `tui_pane` (skeleton from Phase 6; filled in here). The `App.framework: Framework<App>` field-add lands in **Phase 14**, when the framework panes' input paths replace the old `handle_settings_key` / `handle_keymap_key`. Before Phase 14 the filled-in framework type is exercised only by `tui_pane`'s own `cfg(test)` units and `tui_pane/tests/` integration files.

**Widen `pane_order()` from `pub(super)` to `pub` in Phase 11.** Phase 10 shipped it as `pub(super)` (only the dispatcher needed it). Phase 13's bar renderer and Phase 23's `NextPane`/`PrevPane` regression tests in `tui_pane/tests/` need to observe registration order through the public surface. Rename consideration: keep the name `pane_order()` — it returns `&[Ctx::AppPaneId]` and the meaning is exact.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use panes::{KeymapPane, SettingsPane, Toasts, ToastsAction};`. `panes/mod.rs` is `mod keymap; mod settings; mod toasts; pub use keymap::KeymapPane; pub use settings::SettingsPane; pub use toasts::{Toasts, ToastsAction};` (standing rule 1). New `Framework<Ctx>` getters (`editor_target_path`, `focused_pane_mode`, `dismiss`, etc.) get `#[must_use]` per standing rule 4 where applicable.

### Retrospective

**What worked:**
- Inherent action surface (no `Pane<Ctx>`/`Shortcuts<Ctx>` impl) compiled cleanly first try — the constraint about framework panes lacking `APP_PANE_ID` was real, the chosen escape hatch (per-pane action enum + inherent `defaults`/`handle_key`/`mode`/`bar_slots`/`editor_target`) sat naturally next to the trait surface for app panes.
- Reusing the already-shipped `close_overlay()` inside the new `dismiss()` chain kept the public surface from re-implementing the overlay-clear logic in two places.

**What deviated from the plan:**
- `action_enum!` macro extended to accept per-variant `#[doc]` attributes. The plan used the macro at module level for the three new pane action enums and ran into the workspace's `missing_docs = "deny"` lint on macro-emitted variants. Smallest fix was a backward-compatible grammar tweak (`$( $(#[$vmeta:meta])* $Variant ... )`), not hand-rolling three `Action` impls (the GlobalAction precedent).
- `EditState` enums for `KeymapPane` / `SettingsPane` carry `#[allow(dead_code, reason = "...")]` because Phase 11 ships only the `Browse` arm — Phase 14 transitions into `Awaiting` / `Conflict` / `Editing`. Plan didn't predict the lint pressure.
- `keymap_capture_keys` / `settings_edit_keys` were specified as `fn(KeyBind, &mut Ctx)` stubs; clippy demanded `const fn`. Trivial, but worth noting for future stub helpers.

**Surprises:**
- `Framework::new` was not const fn before Phase 11 (Phase 10 already broke that with `HashMap::new()`); Phase 11's pane-field defaults are const-eligible but the function as a whole stays non-const because of the existing `HashMap`. The "frozen verbatim" Phase 6 mirror block is more aspirational than literal — call it "frozen signature, body grows."
- The plan's `bar_slots()` signature for framework panes returns `Vec<(BarRegion, BarSlot<KeymapPaneAction>)>` (etc.) — concrete types, not the trait's `Vec<(BarRegion, BarSlot<Self::Actions>)>`. Phase 13's bar renderer adapter has to special-case each pane's concrete action type, but that was already implicit in the plan's "bar renderer special-cases framework panes" wording.
- The `Vec<String>` message stack on `Toasts<Ctx>` is a placeholder. Cargo-port's `ToastManager` (`src/tui/toasts/manager.rs:236`) already owns the real toast subsystem (IDs, timing, viewport, hitboxes, tracked items, dismiss semantics); a generic typed manager belongs in `tui_pane` and the framework should own toast data, not delegate to the binary. Phase 12 takes that next step.

**Implications for remaining phases:**
- Phase 12 (Framework Toasts skeleton): replaces the `Vec<String>` placeholder with a typed `Toast` manager, splits `FrameworkPaneId` into focus/overlay enums, drops `ToastsAction::Dismiss` (dismiss flows through `GlobalAction::Dismiss`), replaces `Framework::dismiss(&mut self)` with `dismiss_framework(&mut self) -> bool` plus a free `dismiss_chain<Ctx>(ctx, fallback) -> bool`, rewrites `focus_step` to include Toasts as a virtual cycle entry when `has_active()` returns true, and adds `Mode::Navigable` for focused Toasts.
- Phase 13 (bar renderer): the adapter needs concrete-type arms per overlay pane (`KeymapPaneAction`, `SettingsPaneAction`); cannot be one generic helper. The Toasts arm renders nav + toast-pane actions + global once Phase 12 lands the typed manager.
- Phase 15 (reroute overlay input handlers): the `EditState` allow-dead_code blocks come off as soon as `handle_key` constructs `Awaiting` / `Editing` / `Conflict`. Later phases replace the initial text-input bridge with command-returning pane methods (`SettingsPane::handle_text_input`, `KeymapPane::handle_capture_key`).
- Phase 13+ tests: framework-pane snapshot tests will exercise `EditState::Awaiting` / `Editing` / `Conflict` — those phases need to construct the pane in those states (no public setter today; consider `pub(crate)` constructors, or expose a Phase-15 method that drives the transition).
- The `action_enum!` macro grammar widening is a permanent API surface change — Phase 14's binary-side `action_enum!` invocations now use the per-variant `#[doc]` / `#[allow(...)]` attribute form.

### Phase 11 Review

- **Phase 13 — Bindings::entries widening dropped.** Plan previously called for widening `Bindings::entries` from `pub(super)` to `pub(crate)` so `bar/` could read keys for framework panes. Phase 11 ships `defaults()` as **public** on each framework pane, so the bar adapter calls `pane.defaults().into_scope_map()` and uses the public `ScopeMap::key_for` / `display_keys_for` accessors instead. `Bindings::entries` stays `pub(super)`.
- **Phase 13 — concrete-type arms confirmed.** The bar adapter walks three concrete arms (`KeymapPaneAction`, `SettingsPaneAction`, `ToastsAction`) for the framework panes. Phase 12 widens the `ToastsAction` enum (typed manager replaces the placeholder stack) without changing the adapter pattern.
- **Phase 13 — snapshot-test scaffolding called out.** Snapshot tests for `Settings Editing` / `Keymap Awaiting` / `Keymap Conflict` need `cfg(test)` (or `pub(crate)` test-only) constructors on the panes since Phase 11's `EditState` is private and only `Browse` is reachable through the public `new()`. Added an explicit subsection.
- **Phase 13 — `editor_target` deferral noted.** `KeymapPane::editor_target()` and `SettingsPane::editor_target()` always return `None` until Phase 15 wires the transitions; snapshot fixtures must construct only `Browse`-state panes unless they synthesize state per the new test scaffolding.
- **Phase 15 — `EditState` production transitions named explicitly.** Phase 15's body now spells out the `Browse → Editing` / `Browse → Awaiting → Conflict` / cleanup-of-`#[allow(dead_code)]` work; Phase 25/28 later move the text-input mutation state fully into the framework panes.
- **Phase 18 — Esc preflight ordering vs. Phase 11 dispatch chain clarified.** With toasts focused and `example_output` non-empty, Esc fires the structural preflight (clears output) rather than `framework.dismiss()` (would have popped the toast). Matches today's binary; explicit note added.
- **Phase 6 → Phase 11 mirror block softened.** "Frozen verbatim" was always aspirational on `Framework::new`'s body — Phase 10 added `HashMap::new()`, dropping the `const fn` qualifier; Phase 11 adds the three pane defaults. Wording at lines 1249, 1982, and 2095 now reads "frozen signatures + four `const fn` getters/setters; `new` body grows."
- **`action_enum!` macro grammar widening:** per-variant `#[doc]` / `#[allow(...)]` attributes are now part of the documented grammar (required under the workspace's `missing_docs = "deny"` for any public action enum).
- **`Framework<Ctx>` post-Phase-11 surface:** Phase-10-shipped fields (`mode_queries`, `pane_order`, `overlay`) plus Phase-11 additions (three pane fields, `dismiss`, `editor_target_path`, overlay-first `focused_pane_mode`).
- **Toasts placeholder narrowed; framework-owned redesign moves to Phase 12.** `Toasts<Ctx>` ships as a `Vec<String>` message stack with `ToastsAction::Dismiss`, `Mode::Static`, and `Framework::dismiss(&mut self)` as the dismiss method — the minimum viable framework pane. Investigation (cargo-port `ToastManager` at `src/tui/toasts/manager.rs:236`) confirms a real toast subsystem belongs in `tui_pane`: the framework should own toast data, lifecycle, viewport, hitboxes, and dismiss semantics. Phase 12 replaces the placeholder with the typed manager, splits `FrameworkPaneId` into focus/overlay enums, and rewires the focus cycle and dismiss chain accordingly.

### Phase 12 — Framework Toasts skeleton ✅

Phase 12 pivots `Toasts<Ctx>` from the Phase 11 placeholder to a framework-owned typed pane that owns the toast data model. The work splits into five connected pieces, all landing in this phase:

**1. Split `FrameworkPaneId` into overlay and focus enums.** The unified Phase 6 / Phase 11 `FrameworkPaneId { Keymap, Settings, Toasts }` lets the system express invalid states — `overlay = Some(Toasts)` is meaningless (toasts are not an overlay), and `FocusedPane::Framework(Keymap | Settings)` is unreachable post-overlay-switch. Phase 12 splits them so the type system rules those out by construction:

```rust
pub enum FrameworkOverlayId { Keymap, Settings }
pub enum FrameworkFocusId   { Toasts }

pub enum FocusedPane<AppPaneId> {
    App(AppPaneId),
    Framework(FrameworkFocusId),
}
```

`Framework<Ctx>::overlay` now carries `Option<FrameworkOverlayId>`; `Framework<Ctx>::focused` carries `FocusedPane<Ctx::AppPaneId>` over the new `FrameworkFocusId`. Every match site in `framework/mod.rs`, `framework/dispatch.rs`, the bar renderer's overlay arm (Phase 13), and the binary's existing `FrameworkPaneId` references update in lockstep. Re-exports at `tui_pane/src/lib.rs` add `FrameworkOverlayId` and `FrameworkFocusId`; the unified `FrameworkPaneId` is deleted.

**2. Replace the `Vec<String>` placeholder with a typed `Toast` manager.** `Toasts<Ctx>` owns a `Vec<Toast<Ctx>>` plus a viewport cursor for focused-toast navigation. `Toast<Ctx>` is generic over the same `Ctx` from the start — Phase 24 adds the `action: Option<Ctx::ToastAction>` field, Phase 26 adds the lifecycle fields (`lifetime`, `phase`, `tracked_items`). The public type signature does not change across phases; only the field set grows.

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ToastId(u64);

pub struct Toast<Ctx: AppContext> {
    id:    ToastId,
    title: String,
    body:  String,
    style: ToastStyle,  // Normal | Warning | Error
    _ctx:  PhantomData<fn(&Ctx)>,
}

pub struct Toasts<Ctx: AppContext> {
    toasts:   Vec<Toast<Ctx>>,
    viewport: Viewport,   // focus cursor + scroll position
    next_id:  u64,
    _ctx:     PhantomData<fn(&mut Ctx)>,
}
```

Public surface:

```rust
impl<Ctx: AppContext> Toasts<Ctx> {
    pub const fn new() -> Self;
    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId;
    pub fn push_styled(&mut self, title: …, body: …, style: ToastStyle) -> ToastId;
    pub fn dismiss(&mut self, id: ToastId) -> bool;
    pub fn dismiss_focused(&mut self) -> bool;
    pub fn focused_id(&self) -> Option<ToastId>;
    pub fn has_active(&self) -> bool;
    pub fn active(&self) -> &[Toast<Ctx>];
    /// Move the viewport to the first toast — called by `focus_step`
    /// on `Next`-direction entry into Toasts focus.
    pub fn reset_to_first(&mut self);
    /// Move the viewport to the last toast — called by `focus_step`
    /// on `Prev`-direction entry into Toasts focus.
    pub fn reset_to_last(&mut self);
    /// Resolved-nav entry point. Dispatch translates the app's
    /// resolved navigation action via `Navigation::list_navigation`
    /// (default impl matches against the trait's `UP`/`DOWN`/`HOME`/
    /// `END` constants) before calling this method. Pure pane-local
    /// mutation (viewport scroll); no `&mut Ctx` borrow needed.
    pub fn on_navigation(&mut self, nav: ListNavigation) -> KeyOutcome;
    /// Pre-globals hook. Dispatch calls this when the inbound key
    /// maps to `GlobalAction::NextPane`/`PrevPane` and Toasts is
    /// focused. Returns `true` when there is internal scroll room
    /// (consumes the key, blocks the cycle advance). Mirrors
    /// cargo-port's existing "Tab scrolls within the toast list
    /// before advancing focus" behavior, but driven by the keymap
    /// entry for `NextPane`, not literal `Tab` — so a rebound
    /// `NextPane` keeps the consume-while-scrollable behavior.
    pub fn try_consume_cycle_step(&mut self, direction: Direction) -> bool;
    /// No-op wrapper retained for tests that drive raw key dispatch.
    /// Production path uses `on_navigation` + `try_consume_cycle_step`.
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome;
    pub fn mode(&self, ctx: &Ctx) -> Mode<Ctx>;
    pub fn defaults() -> Bindings<ToastsAction>;
    pub fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)>;
}
```

`ToastsAction::Dismiss` from Phase 11 is removed. Toast dismiss flows through `GlobalAction::Dismiss` (cargo-port binds `x` at `src/keymap.rs:409`) — the framework's single dismiss action. The remaining toast-pane action set is empty in Phase 12 (Phase 24 adds `Activate`); focus-internal navigation routes through the `NavigationAction` scope, not a Toasts-local action enum:

```rust
pub enum ToastsAction { /* empty in Phase 12; Phase 24 adds Activate */ }
```

Toast viewport movement is keymap-driven, not literal-key-driven: dispatch resolves the inbound key against the app's `Navigation` scope (via the keymap), translates the resolved action into `ListNavigation` via `Navigation::list_navigation` (default impl matches the action against the trait's `UP`/`DOWN`/`HOME`/`END` constants), and calls `framework.toasts.on_navigation(list_nav)`. A rebound `Navigation::Down` (e.g. to `j`) moves the toast viewport in lockstep with the bar's display key. The cycle-step pre-hook (`try_consume_cycle_step`) consults the live keymap entry for `GlobalAction::NextPane`/`PrevPane` so the same rebinding flows through to the Tab-scrolls-before-advance behavior. Dismiss-on-toast does not flow through these paths — it routes through `GlobalAction::Dismiss → dismiss_chain → dismiss_framework → toasts.dismiss_focused()`. Focused Toasts is `Mode::Navigable` (not `Mode::Static`).

**3. `Framework::dismiss(&mut self)` becomes `dismiss_framework(&mut self) -> bool`; introduce free `dismiss_chain<Ctx>`.** With the framework owning toast dismiss directly, the chain operates purely on `&mut self`:

```rust
impl<Ctx: AppContext> Framework<Ctx> {
    /// Run the framework dismiss chain. Returns `true` when something
    /// was dismissed at the framework level.
    /// 1. Focused toast → `toasts.dismiss_focused()`.
    /// 2. Open overlay → `close_overlay()`.
    /// 3. Otherwise return `false`.
    pub fn dismiss_framework(&mut self) -> bool {
        if matches!(self.focused, FocusedPane::Framework(FrameworkFocusId::Toasts))
           && self.toasts.dismiss_focused()
        {
            return true;
        }
        self.close_overlay()
    }
}

/// Free dispatcher called from `dispatch_global::Dismiss`. Kept as a
/// free fn so the binary's optional `dismiss_fallback` hook (and the
/// focus reconciler) receive `&mut Ctx` after the framework borrow is
/// dropped (no aliasing between `&mut Framework` and `&mut Ctx`).
pub(crate) fn dismiss_chain<Ctx: AppContext>(
    ctx: &mut Ctx,
    fallback: Option<fn(&mut Ctx) -> bool>,
) -> bool {
    if ctx.framework_mut().dismiss_framework() {
        // Framework borrow has dropped; route focus repair through
        // ctx.set_focus so any binary-side AppContext::set_focus
        // override (Focus subsystem, telemetry) sees the transition.
        reconcile_focus_after_toast_change(ctx);
        return true;
    }
    if let Some(hook) = fallback { hook(ctx) } else { false }
}
```

The `dispatch_global::Dismiss` arm becomes a one-liner: `dismiss_chain(ctx, keymap.dismiss_fallback_hook())`.

**4. Rewrite `focus_step` to include Toasts as a virtual cycle entry.** Phase 11 leaves Toasts unreachable from the Tab cycle (`focus_step` early-returns on any `FocusedPane::Framework(_)`). Phase 12 builds a per-call cycle that appends `Framework(FrameworkFocusId::Toasts)` when `framework.toasts.has_active()` returns `true`, mirroring cargo-port's `tabbable_panes()` (`src/tui/app/mod.rs:795-809`):

```rust
fn focus_step<Ctx: AppContext>(ctx: &mut Ctx, direction: i32) {
    let cycle = focus_cycle(ctx);
    if cycle.is_empty() { return; }

    let current = *ctx.framework().focused();
    let len = i32::try_from(cycle.len()).unwrap_or(i32::MAX);
    let next = match cycle.iter().position(|p| *p == current) {
        Some(idx) => {
            let cur = i32::try_from(idx).unwrap_or(0);
            cycle[((cur + direction).rem_euclid(len)) as usize]
        }
        None => if direction > 0 { cycle[0] } else { cycle[cycle.len() - 1] },
    };

    let entering_toasts =
        matches!(next,    FocusedPane::Framework(FrameworkFocusId::Toasts))
     && !matches!(current, FocusedPane::Framework(FrameworkFocusId::Toasts));
    ctx.set_focus(next);
    if entering_toasts {
        if direction > 0 { ctx.framework_mut().toasts.reset_to_first(); }
        else             { ctx.framework_mut().toasts.reset_to_last();  }
    }
}

fn focus_cycle<Ctx: AppContext>(ctx: &Ctx) -> Vec<FocusedPane<Ctx::AppPaneId>> {
    let mut cycle: Vec<_> = ctx
        .framework()
        .pane_order()
        .iter()
        .copied()
        .map(FocusedPane::App)
        .collect();
    if ctx.framework().toasts.has_active() {
        cycle.push(FocusedPane::Framework(FrameworkFocusId::Toasts));
    }
    cycle
}
```

Key invariants:
- `pane_order()` continues to carry only `Ctx::AppPaneId` entries.
- The cycle order matches cargo-port's `tabbable_panes()`: app panes by registration, Toasts last when active.
- Direction-aware fallback (`Next → first`, `Prev → last`) when current focus is not in the cycle.
- `reset_to_first` / `reset_to_last` on entry replace cargo-port's `viewport.home()` / `viewport.set_pos(last_index)` calls in `focus_next_pane`/`focus_previous_pane`.
- Tab-as-cycle-step consume runs as a pre-globals hook (`try_consume_cycle_step`) that consults the live keymap entry for `GlobalAction::NextPane`/`PrevPane` — not literal `Tab` — so a rebound `NextPane` keeps the consume-while-scrollable behavior. The hook returns `true` when there is internal scroll room (consumes the keystroke, blocks the cycle advance); otherwise dispatch falls through to globals and the cycle advances.

**5. Focus reconciliation after dismiss / prune.** When `Toasts::dismiss(_)` or any Phase-22 prune-on-tick path empties the active set while Toasts is focused, focus moves to the first live app tab stop (or no-op if the live cycle is empty). Reconciliation **must** route through `ctx.set_focus(...)` — not `framework.set_focused(...)` — so binaries that override `AppContext::set_focus` (logging, telemetry, the `Focus` subsystem's overlay-return memory) still observe the transition. That rules out a `&mut self` method on `Framework<Ctx>` (which has no path to `&mut Ctx`); the reconciler is a free fn over `&mut Ctx`:

```rust
// tui_pane/src/framework/dispatch.rs
pub(super) fn reconcile_focus_after_toast_change<Ctx: AppContext>(ctx: &mut Ctx) {
    let framework = ctx.framework();
    if !matches!(framework.focused(), FocusedPane::Framework(FrameworkFocusId::Toasts)) {
        return;
    }
    if framework.toasts.has_active() {
        return;
    }
    let target = framework.pane_order().first().copied().map(FocusedPane::App);
    if let Some(target) = target {
        ctx.set_focus(target);
    }
}
```

`dismiss_chain<Ctx>` calls this after `framework.dismiss_framework()` returns true (and the framework borrow drops). Phase 26's prune-on-tick path calls it after `framework.prune(now)` returns. Both call sites already hold `&mut Ctx` because they live in the dispatch chain or the tick loop. `Framework<Ctx>` itself exposes no focus-write API beyond the Phase 6 `pub(super) fn set_focused` setter; toast-driven focus repair is a dispatcher concern, not a framework-internal mutation.

**Bar render — focused Toasts.** With the typed manager in place, the bar renderer's `FocusedPane::Framework(FrameworkFocusId::Toasts)` arm renders the navigation row and the global region. The `PaneAction` region is empty in Phase 12 (the empty `ToastsAction` enum produces no `bar_slots` entries); Phase 24 fills it with `Activate`. Phase 13's bar `mod.rs` still walks `framework.toasts.bar_slots(ctx)` for the `PaneAction` region — the walk just returns nothing until Phase 24. The exact bar contents:
- `Mode::Navigable` (focused Toasts behave like a list): `Nav` region renders the app's `Navigation::UP` / `Navigation::DOWN` keys (the framework reads them through the keymap by looking up the bindings registered for those trait constants), `PaneAction` is empty (Phase 12 `ToastsAction` is empty; Phase 24 adds `Activate`), `Global` renders `GlobalAction::Dismiss` (and the rest of the global strip).

**Phase 12 tests** (in `tui_pane/tests/` and `tui_pane/src/panes/toasts.rs`):
- `pane_order_empty_and_toasts_active_cycles_to_toasts` — Tab from no-focus state lands on Toasts when no app panes registered.
- `toasts_inactive_while_focused_next_moves_to_app_pane` — when Toasts becomes inactive while focused, the next Tab leaves Toasts cleanly.
- `prev_from_first_app_lands_on_toasts_when_active` — Shift-Tab from the first app pane lands on Toasts.
- `dismiss_focused_toast_removes_it_and_reconciles_focus` — when Toasts becomes empty after a dismiss, focus moves to the first live app tab stop.
- `entering_toasts_with_next_calls_reset_to_first` / `entering_toasts_with_prev_calls_reset_to_last` — viewport reset on entry.
- `dismiss_chain_closes_overlay_when_no_focused_toast` — overlay-only dismiss path.
- `dismiss_chain_falls_through_to_fallback_when_neither_fires` — registered fallback hook is called.
- `bar_slots_for_focused_toasts_includes_nav_and_global` — bar fixture (snapshot lands in Phase 13).

**Code touched in Phase 12** (cargo-port code is unaffected; framework migration of cargo-port's `ToastManager` lands in Phase 26):
- `tui_pane/src/pane_id.rs` — split into `FrameworkOverlayId` + `FrameworkFocusId`; rewrite `FocusedPane`.
- `tui_pane/src/panes/toasts.rs` — replace placeholder with typed manager.
- `tui_pane/src/framework/list_navigation.rs` — new file; defines `pub enum ListNavigation { Up, Down, Home, End }`. Framework-owned, reusable by future framework list panes.
- `tui_pane/src/keymap/navigation.rs` — extend the `Navigation<Ctx>` trait with `const HOME: Self::Actions;`, `const END: Self::Actions;`, and `fn list_navigation(action: Self::Actions) -> Option<ListNavigation>` (default impl matches the action against `UP`/`DOWN`/`HOME`/`END`). Cargo-port's Phase 14 `Navigation` impl supplies the two new constants.
- `tui_pane/src/framework/mod.rs` — `overlay` field type change, `focused_pane_mode` arm rewrite, `dismiss` → `dismiss_framework`.
- `tui_pane/src/framework/dispatch.rs` — `dispatch_global::Dismiss` calls `dismiss_chain`; rewrite `focus_step` per the pseudocode; add `reconcile_focus_after_toast_change<Ctx>(ctx: &mut Ctx)` free fn that routes through `ctx.set_focus(...)` so binary-side `AppContext::set_focus` overrides observe the transition.
- `tui_pane/src/lib.rs` — re-export `FrameworkOverlayId`, `FrameworkFocusId`, `Toast`, `ToastId`, `ToastStyle`, `ListNavigation`; drop `FrameworkPaneId`.
- `tui_pane/src/framework/mod.rs` test module — rewrite cases that named `FrameworkPaneId`.

### Retrospective

**What worked:**
- Five-piece split (id enums / typed manager / dismiss chain / focus cycle / Navigation extension) landed without rework — each piece compiled cleanly against the previous one in order.
- 11 integration tests in `tui_pane/tests/framework_toasts.rs` exercise the full chain through `Keymap::dispatch_framework_global`; one (`focus_changes_route_through_app_context_set_focus`) locks the Phase-19 invariant that focus changes route through `ctx.set_focus(...)`.

**What deviated from the plan:**
- `ToastsAction` was hand-rolled (empty enum + manual `Action` + `Display` impls) because `action_enum!` requires ≥1 variant. The plan's `pub enum ToastsAction { /* empty */ }` snippet is correct but does not flow through the macro.
- Added `CycleDirection { Next, Prev }` (a closed enum) for `Toasts::try_consume_cycle_step`'s `direction` parameter — the plan said `direction: Direction` but did not define `Direction`. Lives next to `ListNavigation` in `framework/list_navigation.rs` and re-exported at the crate root.
- Renamed the inner `toasts` field on `Toasts<Ctx>` to `entries` to clear the `clippy::struct_field_names` lint. Public surface unchanged.
- `Display` for the empty `ToastsAction` returns `Ok(())` rather than `match *self {}` — clippy's `uninhabited_references` flags the deref, and `unreachable!()` is forbidden by the workspace's `clippy::unreachable` lint.
- `framework::list_navigation` is a sub-module of `framework/`, but the public re-exports (`crate::ListNavigation`, `crate::CycleDirection`) flow through `framework/mod.rs`. `framework/list_navigation.rs` itself is not declared at the crate root — matches the existing `framework/dispatch.rs` pattern.

**Surprises:**
- `clippy::option_if_let_else` (nursery) flagged the `match cycle.iter().position(...) { Some(idx) => ..., None => ... }` arm in `focus_step`. Rewrote with `.map_or_else(|| fallback, |idx| advance)` — equivalent control flow, different expression.
- The Display impl for an uninhabited type is genuinely awkward in this lint stack: `match *self {}` is UB-flagged, `unreachable!()` is banned, `match self {}` (without deref) needs nightly `exhaustive_patterns`. Returning `Ok(())` is the only path that compiles clean — and it is sound because the method cannot be called.
- Integration test files at `tui_pane/tests/` need their own `#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]` block; the workspace's lint config does not exempt integration tests automatically.

**Implications for remaining phases:**
- **Phase 13 — bar renderer:** The Toasts arm of the bar adapter calls `framework.toasts.bar_slots(ctx).resolve_keys(...)`, which returns an empty `Vec<RenderedSlot>` in Phase 12 because `ToastsAction::ALL = &[]`. The "concrete arm per pane" walk handles this without a special case; no Phase-13 logic change beyond what the plan already names.
- **Phase 13 — `Mode::Navigable` for focused Toasts:** the plan already states this; Phase 12 confirms `Toasts::mode` returns `Mode::Navigable`, so the `Nav`-region rule (suppress when mode is `Static` / `TextInput` / `None`) emits the nav row for Toasts as expected.
- **Phase 14 — `Navigation<App>` impl needs HOME / END:** cargo-port's `NavigationAction` enum gains `Home` / `End` variants (with TOML keys `"home"` / `"end"`) and bindings (default `KeyCode::Home` / `KeyCode::End`). `list_navigation` default impl takes care of the translation; no per-impl override needed.
- **Phase 15 — focused-Toasts dispatch chain:** Phase 12 owns the input path on the framework side (`dismiss_chain` / `on_navigation` / `try_consume_cycle_step`). Phase 15's "Toasts focus gate" subsection now reduces to wiring the inbound key through the Navigation scope, calling the right framework method, and recognising that `ToastsAction::Dismiss` is gone (Esc-on-Toasts flows through `GlobalAction::Dismiss`).
- **Phase 18 — structural Esc preflight:** unchanged. With Toasts focused and `example_output` non-empty, the preflight still wins; `dismiss_chain` only fires when the preflight does not match the bound key.
- **Phase 26 — toast manager migration:** the typed `Toast<Ctx>` / `Toasts<Ctx>` skeleton is in place. Phase 26 grows the field set (lifecycle, tracked items, phase) and adds the rendering / hitbox / format modules. No struct-level renames; Phase 26 also replaces the private `body: String` storage with a typed `ToastBody` (see Phase 26 §1) — an intentional internal representation change, not purely additive. The `Toasts::active()` slice is read-only and stable.
- **`CycleDirection` is now a public framework type.** Phase 26's prune-on-tick path will use it where the existing `i32 direction` argument was implied; binaries that drive focus programmatically (cargo-port does not, today) get a cleaner enum.

### Phase 12 Review

- **Phase 13** — added one sentence under the "concrete arm per pane" prose noting `Toasts::bar_slots` returns an empty `Vec<RenderedSlot>` in Phase 12 because `ToastsAction::ALL = &[]`; the Nav/Global regions still emit. Phase 24 re-snapshots after `Activate` lands.
- **Phase 14** — `NavigationAction` line now spells out the six variants (`Up`/`Down`/`Left`/`Right`/`Home`/`End`), TOML keys `"home"`/`"end"`, default bindings `KeyCode::Home`/`KeyCode::End`, and that the `Navigation<App>` impl inherits the trait's default `list_navigation` (no override).
- **Phase 15** — Toasts focus gate prose collapsed to a one-line preface ("Framework owns the input path per Phase 12; Phase 15 wires the inbound key through these hooks"). Step 1 now names the typed `CycleDirection::Next` / `CycleDirection::Prev` argument and clarifies that the matched-action branch picks which is passed. Step 2 notes that `dismiss_chain` calls `reconcile_focus_after_toast_change(ctx)` automatically, so dispatch needs no extra Phase-15 call site. Closing line records that `ToastsAction::Dismiss` is gone (Phase 12).
- **Phase 18** — Esc-preflight tradeoff sentence widened to `dismiss_chain → dismiss_framework → toasts.dismiss_focused()` for symmetry with the rest of the doc.
- **Phase 19** — top of the section now states that framework-side cleanup (`FrameworkPaneId`, `Framework::dismiss`, `try_pop_top`, `ToastsAction::Dismiss`, `Vec<String>` placeholder, `Mode::Static` for Toasts) all landed in Phase 12; Phase 19 deletes binary-side artifacts only.
- **Phase 23** — bar-on-rebind list now includes a `key_for(NavigationAction::Home/End)` round-trip assertion. The `AppContext::set_focus is the single funnel` bullet now cross-references Phase 12's `focus_changes_route_through_app_context_set_focus` test (focused-Toasts arm already covered) and frames Phase 23 as widening to overlay-state assertions and pane cycling after the Phase 20-22 cleanup.
- **Phase 24** — §3 now states explicitly that the Phase-12 hand-rolled `Action` / `Display` impls on `ToastsAction` are deleted, and `ToastsAction::Activate` is declared via the standard `action_enum!` macro (`Activate => ("activate", "open", "Activate focused toast")`).
- **Phase 26** — §1 now flags `body: String → body: ToastBody` as an intentional internal representation change (not purely additive), shows the `ToastBody { Line, Lines }` enum with `From<String>` / `From<&str>` impls, and lists which push entry points keep `impl Into<String>` boundary conversion. Adds the `Toast::body()` accessor decision: returns `&ToastBody` (public-API change in this phase). §2 now prefaces the new method list with the Phase 12 / Phase 24 surface so a reader does not assume those methods are missing; `push_timed` / `push_task` arguments clarified to take raw `Duration` (not `ToastDuration` — that newtype validates TOML, not in-code Durations). §5 adds that the dispatch-time call site to `reconcile_focus_after_toast_change` was wired in Phase 12; Phase 26 only adds the tick-driver call site. §6 collapsed to a single-sentence cross-reference. Cross-crate test note clarifies `NoToastAction`-typed test pushes use `action: None` only (the type is uninhabited).
- **Phase 12 retrospective wording** — softened from "field-set growth, no renames" to "no struct-level renames; Phase 26 also replaces the private `body: String` storage with `ToastBody`," matching the friend's review.

### Phase 13 — Framework bar renderer ✅

Add `tui_pane/src/bar/` per the BarRegion model:

- `mod.rs` — `render(focused, ctx, keymap, framework) -> StatusBar`. Matches `focused: &FocusedPane<Ctx::AppPaneId>` first, fetches `Vec<RenderedSlot>` from the right source, walks `BarRegion::ALL`, dispatches to each region module, joins spans into `StatusBar`.
- `region.rs` — `BarRegion::{Nav, PaneAction, Global}` + `ALL` (added Phase 5).
- `slot.rs` — `BarSlot<A>`, `ShortcutState`, `BarSlot::primary` (added Phase 5 / 9).
- `support.rs` — `format_action_keys(&[KeyBind]) -> String`, `push_cancel_row`, shared row builders.

**Top-level dispatch is overlay-first, then `FocusedPane`.** Overlays render their own bar; otherwise app panes flow through the keymap and the `Framework(Toasts)` focus-state arm reads from `framework.toasts.bar_slots(ctx)`. Overlays render *over* the underlying pane, mirroring the binary's existing keymap/settings overlay rendering:

```rust
let pane_slots: Vec<RenderedSlot> = match framework.overlay() {
    Some(FrameworkOverlayId::Keymap)   => framework.keymap_pane  .bar_slots(ctx).resolve_keys(...),
    Some(FrameworkOverlayId::Settings) => framework.settings_pane.bar_slots(ctx).resolve_keys(...),
    None => match focused {
        FocusedPane::App(id) => keymap.render_app_pane_bar_slots(*id, ctx),
        FocusedPane::Framework(FrameworkFocusId::Toasts) =>
            framework.toasts.bar_slots(ctx).resolve_keys(...),
    },
};
// region modules then partition pane_slots by `region` field.
```

The three framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) each return a concrete `Vec<(BarRegion, BarSlot<{Pane}Action>)>` from their inherent `bar_slots()`. The bar adapter walks one concrete arm per pane, not one generic helper. Each pane's `defaults()` is **public** (Phase 11), so the adapter calls `pane.defaults().into_scope_map()` and uses the public `ScopeMap::key_for` / `display_keys_for` accessors to pair labels with keys — no `Bindings::entries` widening needed. `framework_globals` resolves through the existing `Keymap::framework_globals()` accessor (which returns `&ScopeMap<GlobalAction>` directly — typed, no downcast). The result: every region module sees `Vec<RenderedSlot>` regardless of source. In Phase 12's state `Toasts::bar_slots` returns an empty `Vec<RenderedSlot>` (because `ToastsAction::ALL = &[]`); the `Nav` and `Global` regions still emit. Phase 24 re-snapshots once `Activate` lands and `Toasts::bar_slots` produces a non-empty `PaneAction` row.

**Editor-target wire-deferred.** `KeymapPane::editor_target()` and `SettingsPane::editor_target()` ship in Phase 11 but always return `None` until Phase 15 wires the `Awaiting`/`Editing` transitions. Phase 13's snapshot fixtures construct only `Browse`-state framework panes unless they synthesize the editor state per the test scaffolding called out below.

**Snapshot-test scaffolding for non-`Browse` states.** Phase 13 needs to render `Settings Editing`, `Keymap Awaiting`, and `Keymap Conflict` for snapshot coverage, but Phase 11 ships `EditState` as a private enum with only `Browse` reachable via `KeymapPane::new()` / `SettingsPane::new()`. Phase 13 adds a `cfg(test)` (or `pub(crate)` test-helper) constructor on each overlay pane — e.g. `KeymapPane::for_test(EditState::Awaiting, Some(path))` — so snapshot fixtures can place a pane in any state without going through Phase 15's not-yet-shipped key-transition path. The `#[allow(dead_code, reason = "Phase 15 transitions...")]` on the variants comes off in Phase 15 once the production transitions land.

**Region modules walk `RenderedSlot { region, .. }`, not typed `BarSlot<A>` tuples.** With Phase 9's `RenderedSlot` carrying `region: BarRegion` as a flat field, the per-region modules filter by field-match — they no longer thread an `A` type parameter:

- `nav_region.rs` — emits framework's nav + pane-cycle rows when `matches!(framework.focused_pane_mode(ctx), Some(Mode::Navigable))`, then `pane_slots.iter().filter(|s| s.region == BarRegion::Nav)`. Suppressed entirely when the mode is `Static`, `TextInput(_)`, or `None` (no pane registered for the focused id).
- `pane_action_region.rs` — emits `pane_slots.iter().filter(|s| s.region == BarRegion::PaneAction)`. Renders for `Some(Mode::Navigable)` and `Some(Mode::Static)`; suppressed for `Some(Mode::TextInput(_))` and `None`.
- `global_region.rs` — emits `GlobalAction` + `AppGlobals::render_order()` (resolved through the same `RenderedSlot` adapter); suppressed when `matches!(framework.focused_pane_mode(ctx), Some(Mode::TextInput(_)))`.

Depends on Phase 12 (typed `Toasts<Ctx>` manager, split `FrameworkOverlayId` / `FrameworkFocusId`) plus Phase 9's `Keymap<Ctx>` lookups.

Snapshot tests in this phase cover the framework panes only (Settings Browse / Settings Editing / Keymap Browse / Keymap Awaiting / Keymap Conflict / Toasts focused) plus a fixture pane exercising every `BarRegion` rule. The Toasts snapshot fixture exercises the typed manager: nav row from the `Navigation` scope (translated to `ListNavigation`), an empty `PaneAction` region (Phase 12 has no toast-local actions), and the global region with `GlobalAction::Dismiss`. Phase 24 re-snapshots once `Activate` lands. App-pane snapshots land in Phase 14 once their `Shortcuts<App>` impls exist.

**Paired-row separator policy.** Inherited from the Phase 2 retrospective decision: the `Paired` row's `debug_assert!` covers only the parser-producible `KeyCode` set; widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep. See Phase 2 review block (line 1020) for full text.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::StatusBar;` (and any other public bar types not already exported in Phase 5). All `bar/` submodules declared `mod` (private) in `bar/mod.rs` per standing rule 1.

#### Retrospective

**What worked:**
- Region modules (`nav_region.rs`, `pane_action_region.rs`, `global_region.rs`) plus `support.rs` row builders kept each region's suppression rule local. `bar::render` reduces to a `BarRegion::ALL` walk that delegates to module-level free fns.
- `cfg(test) pub(crate)` `for_test_*` constructors on `KeymapPane` / `SettingsPane` plus `Framework::set_overlay_for_test` let the in-crate snapshot tests place panes in `Awaiting` / `Conflict` / `Editing` directly. 14 unit tests in `bar/tests.rs` lock the rules; 3 integration tests in `tui_pane/tests/framework_bar.rs` lock the public path.
- 600/600 workspace tests pass; clippy + fmt clean.

**What deviated from the plan:**
- **Type-erased renderer fn pointers added to `Keymap<Ctx>`.** The plan said the bar renderer "is generic over no `<A>` parameter" but didn't spell out the mechanism. Phase 13 adds two `fn(&Keymap<Ctx>) -> Vec<RenderedSlot>` slots to `Keymap` (`navigation_render_fn`, `app_globals_render_fn`) plus a `ScopeRenderFn<Ctx>` type alias mirrored on the builder. `KeymapBuilder::register_navigation::<N>` / `register_globals::<G>` capture the `<N>`/`<G>`-monomorphized `runtime_scope::render_navigation_slots` / `render_app_globals_slots` free fn at register time and copy it onto the keymap in `finalize`. This is the answer to "how does the bar reach the binary's nav/globals scopes without naming the action enum."
- **New public `Keymap` accessors:** `render_navigation_slots`, `render_app_globals_slots`, `render_framework_globals_slots`. The first two delegate to the stored fn pointer (empty `Vec` if not registered); the third works directly off `framework_globals` because `GlobalAction` is concrete. The bar consumes all three.
- **`bar::render` re-exported as `tui_pane::render_status_bar`.** Crate-root rename (`pub use bar::render as render_status_bar;`) so the binary's call site reads as a verb, not as an unqualified `render` clash.
- **Pane-cycle pair filtered out of the global region.** `nav_region` emits `NextPane`/`PrevPane` as a paired `"Tab/Shift+Tab pane"` row; `global_region` walks `GlobalAction::ALL` and drops those two so they don't render twice. The drop is by `bar_label` reverse-lookup against `GlobalAction::ALL` — accepted O(n²) in the global slot count to avoid putting a region tag on every variant.
- **Toasts arm renders empty pane-action slots.** Phase 12 ships `ToastsAction` empty (uninhabited) so the resolver short-circuits to `Vec::new()` rather than walking `Toasts::bar_slots`. Phase 24 widens the enum and the arm starts producing entries.

**Surprises:**
- **`ToastsAction` being uninhabited triggers dead-code inference.** A `filter_map(|(region, slot)| { let action = slot.primary(); ... })` body on `Vec<(BarRegion, BarSlot<ToastsAction>)>` warns "unused variables" because `slot.primary(): ToastsAction` is `!`-like and the compiler eliminates the closure body. Working around it with `Vec::new()` for the Toasts arm is cleaner than `_action` / `_region`.
- **The plan said `cfg(test)` constructors would suffice.** They do for unit tests, but `cfg(test)` is invisible to integration tests in `tui_pane/tests/`. The split (overlay-edit-state coverage in `bar/tests.rs`, public-path coverage in `tests/framework_bar.rs`) is mandatory; collapsing them would force a `pub` test surface or a `test-helpers` feature.
- **`Mode<Ctx>` is `!Eq` because `TextInput(fn(KeyBind, &mut Ctx))` carries a fn pointer.** Tests had to use `matches!(bar, ...)` instead of `bar == ...` for assertions. The plan didn't predict this would matter; in practice, every region module already pattern-matches on `Mode<Ctx>` so it never came up.

**Implications for remaining phases:**
- **Phase 14 binary integration: wire the call site, supply the palette.** The binary's main render path replaces its current `shortcuts::for_status_bar(...)` + `shortcut_spans` glue with the framework bar renderer. Phase 26 closeout moved the final status-line ownership into `tui_pane::render_status_line(...)`: the framework fills the line, places left/center/right regions, renders uptime / scanning text, resolves global shortcut keys, and styles every slot. The binary supplies only app facts (`uptime_secs`, `scanning`) plus a `StatusLineGlobal` policy array for which global actions appear and whether app-dependent entries are enabled.
- **Phase 14 binary cleanup is larger than the plan named.** With `tui_pane::render_status_bar` available, the binary's `src/tui/shortcuts.rs::for_status_bar` (the giant match on `InputContext`) and `src/tui/render.rs::render_status_bar`'s shortcut-spans plumbing can both retire — but Phase 14 keeps the parallel-path invariant, so the deletion happens in Phase 19, not 14. Phase 14 just adds the new call site beside the old one.
- **Phase 23 `Phase 23 — Regression tests` is the right scope.** Rebinding tests already in the plan (`*Action::Activate` rebound updates pane bar; `NavigationAction::Up`/`Down` rebound updates the nav row; `GlobalAction::NextPane` rebound updates pane-cycle) all work against the public `render_status_bar` surface — the type-erased renderer fn pointers mean rebinding TOML changes the rendered keys without any dispatch-side test scaffolding.
- **Bar styling: framework styles, binary supplies palette (post-Phase-13 amendment).** Phase 13 ships unstyled `Span::raw(...)`. The post-review decision: the framework owns the styling pass, the binary owns the palette. Phase 14 adds a public `BarPalette` type to `tui_pane::bar` (`enabled_key_style`, `enabled_label_style`, `disabled_key_style`, `disabled_label_style`, `separator_style`) and widens `render_status_bar` to take `&BarPalette` as its fifth argument. `support::push_slot` / `push_paired` consume the palette to style each `Span`; `slot.state` (currently discarded) drives the enabled-vs-disabled style selection. The framework ships **no `Default` palette that bakes in cargo-port colors** — any `BarPalette::default()` (if added) is plain `Style::default()` for every field, neutral and theme-agnostic. Cargo-port supplies a `cargo_port_bar_palette()` constructor inside the binary that wires `ACCENT_COLOR` / `SECONDARY_TEXT_COLOR` / `Modifier::BOLD` to match the pre-refactor look.
- **The framework's overlay-vs-app-globals contrast is now visible.** Phase 13's `pane_action_region` renders Settings Browse's Edit/Save/Cancel slots on `Mode::Navigable`, and `global_region` renders the framework + app globals on the same focus. The existing binary's `for_status_bar` blanket-suppresses globals on overlays. Phase 14's parallel test path will surface a behavior diff for any binary code that was relying on the blanket suppression — flag it during Phase 14 review rather than treating it as a Phase 13 retrospective bug.

#### Phase 13 Review

- **Phase 14 (App action enums + `Shortcuts<App>` impls)** — added explicit "wire the call site, supply the palette" framing on the binary integration bullet; added two new deliverables: introduce `BarPalette` in `tui_pane/src/bar/palette.rs` (re-exported at crate root), widen `render_status_bar` to take `&BarPalette` as its fifth argument, and ship a `cargo_port_bar_palette()` constructor on the binary side wiring `ACCENT_COLOR` / `SECONDARY_TEXT_COLOR` / `Modifier::BOLD` to match the pre-refactor bar exactly. `BarPalette::default()` is theme-neutral (no cargo-port colors in the framework). Added an explicit "Builder call order" block showing `register_navigation::<AppNavigation>()` / `register_globals::<AppGlobalAction>()` precede the first `register::<Pane>(...)`.
- **Phase 19 (Bar swap and cleanup)** — extended deletion list to include `src/tui/render.rs::shortcut_spans` and `shortcut_display_width` (the binary's pre-refactor styling/flattening glue, obsoleted by `BarPalette`); kept `cargo_port_bar_palette()` (theme code stays binary-side).
- **Phase 23 (Regression tests)** — reworded the snapshot parity claim from "byte-identical bar output" to "the new static-label framework bar produced by `render_status_bar` + `cargo_port_bar_palette()` under default bindings" (Phase 14.1 follow-up). Phase 14 deliberately collapses today's row-dependent labels (e.g. `PackageAction::Activate`'s `"URL"`/`"Cargo.toml"` switch) into one static `bar_label` per variant; Phase 23 snapshots lock the new bar, not the pre-refactor bar. Also rewrote the `key_for(NavigationAction::Home) == KeyCode::Home` test bullet to use `keymap.navigation::<AppNavigation>().expect(...).key_for(NavigationAction::Home).copied() == Some(KeyCode::Home.into())` (the typed singleton getter, since the public bar surface is type-erased).
- **Phase 24 (Toast activation payload)** — added an explicit step to remove the Phase-13 `Vec::new()` short-circuit in `tui_pane/src/bar/mod.rs::pane_slots_for`'s Toasts arm and replace it with the standard resolver pattern once `ToastsAction::Activate` lands (the dead-code closure inference issue evaporates with a populated enum).
- **Phase 26 (`ToastManager` migration)** — added a constraint that the storage move preserve `bar_slots` / `mode` / `defaults` public signatures verbatim, since the bar resolver in `tui_pane/src/bar/mod.rs` depends on them.
- **README inventory ("What survives" block)** — added the public bar surface (`StatusBar`, `BarPalette`, `render_status_bar` signature, the three `Keymap::render_*_slots` accessors) and named the framework as the styling-pass owner.
- **Reviewed and not changed:** Phase 19 deletion list (subagent finding 4 — already correct, only the `shortcut_spans` / `shortcut_display_width` addition was new); Phase 25 `Framework::new` constructor stability (subagent finding 10 — confirmed unchanged by Phase 13); Phase 26 push-API additions (subagent finding 11 — `Toasts::push` signature stable, no caller in `bar/tests.rs` relies on a `String` body type); Phase 23's `set_focus` funnel test (subagent finding 16 — orthogonal to Phase 13).
- **Known follow-up:** `bar/global_region.rs::framework_action_for_label` does an O(n²) reverse lookup against `GlobalAction::ALL` (n = 7) per render to drop `NextPane`/`PrevPane` from the global region. Bounded cost. A future optimization can add a region-discriminator to `RenderedSlot` (or split the framework-globals renderer into nav-cycle vs global halves) if profiling justifies it.

### Phase 14 — App action enums + `Shortcuts<App>` impls

**Parallel-path invariant for Phases 14–18.** The new dispatch path lands alongside the old one. The old path stays the source of truth for behavior through Phase 17; Phase 18 enables the narrow structural Output-cancel framework read after TOML loading lands. **Phase 19 is the only phase that deletes** old dispatch code.

**Flat-namespace paths (per Phase 5+ standing rule 2).** Every `tui_pane` import in this phase uses flat paths: `use tui_pane::KeyBind;`, `use tui_pane::GlobalAction;`, `use tui_pane::Shortcuts;`, `tui_pane::action_enum! { ... }`, `tui_pane::bindings! { ... }`. Never `tui_pane::keymap::Foo`.

**Binary-side `mod` rule (per Phase 5+ standing rule 1).** New module files added to `src/tui/` for the new action enums (e.g. `app_global_action.rs`, `navigation_action.rs`) are declared `mod foo;` at their parent (never `pub mod foo;`); facades re-export with `pub use foo::Type;`. `cargo mend` denies `pub mod` workspace-wide.

In the cargo-port binary crate:

- **`action_enum!` migration cost.** Every existing `action_enum!` invocation in the binary gains a third positional `bar_label` literal between the toml key and description, per Phase 5's grammar amendment. The hand-rolled `tui_pane::GlobalAction` already ships its own `bar_label` (Phase 5). The macro itself was already updated in Phase 5 and the cross-crate fixtures in `tui_pane/tests/macro_use.rs` already use the 3-positional form (verified Phase 7) — Phase 14's binary-side migration is purely a per-call-site update, not a grammar change. **Note (Phase 14.1 audit):** the pre-refactor binary defines its action enums in `src/keymap.rs`, not `src/tui/`, via a local `action_enum!` macro with a 2-positional grammar; Phase 14's migration also flips those invocations to `tui_pane::action_enum!` and gains the bar_label literal in the same edit.
- **`bar_label` is one static literal per variant.** The framework's contract is `Action::bar_label(self) -> &'static str` — one value per variant, no `&self` or `&Ctx`. Today's binary renders row-dependent labels (e.g. `PackageAction::Activate` shows `"URL"` on the Repository row vs `"Cargo.toml"` on the Manifest row); Phase 14 collapses each variant to one short generic label (`"activate"` / `"clean"` / `"open"` / `"clear"` / etc.). Row-dependent labels are not preserved unless a deliberate split into new action variants happens at the same time. Phase 23's snapshots therefore lock the new static-label framework bar — they do **not** assert byte-identical equality with the pre-refactor dynamic-label bar.
- Define `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction`. **`NavigationAction` carries six variants** (`Up`, `Down`, `Left`, `Right`, `Home`, `End`) per the Phase 12 trait extension; the new `Home` / `End` variants take TOML keys `"home"` / `"end"` and default bindings `KeyCode::Home` / `KeyCode::End`. The cargo-port `Navigation<App>` impl supplies `const HOME` / `const END` and inherits the trait's default `list_navigation` (no per-impl override — the four directional consts plus `HOME` / `END` are sufficient).
- **Prepare the globals split without deleting the legacy enum.** During Phases 14–18, cargo-port carries both globals surfaces: the legacy binary `crate::keymap::GlobalAction` continues powering the old dispatch path, while the new framework keymap uses `tui_pane::GlobalAction` for framework-owned globals (Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss) plus `AppGlobalAction` for binary-owned globals. New framework-global references are path-qualified as `tui_pane::GlobalAction` to disambiguate. Phase 19 makes the framework dispatch path live; Phase 20 deletes the legacy binary enum once the keymap viewer/editor no longer depend on `ResolvedKeymap`.
- Add `ExpandRow` / `CollapseRow` to `ProjectListAction`.
- Implement `Pane<App>` and `Shortcuts<App>` for each app pane (Package, Git, ProjectList, CiRuns, Lints, Targets, Output, Lang, Cpu, Finder). Each pane:
  - `Pane<App>` block declares `const APP_PANE_ID: AppPaneId` and overrides `mode()` when needed (FinderPane returns `Mode::TextInput(finder_keys)` while open, else `Mode::Navigable`; OutputPane returns `Mode::Static`; the rest accept the default `Mode::Navigable`). **Override body uses the free-fn signature** — `fn mode() -> fn(&App) -> Mode<App> { |ctx| ... }` — and the closure reads state by navigating from `ctx: &App` (e.g. `ctx.overlays.finder.is_open()`), never `&self`. **No per-impl `#[must_use]`**: the trait declaration carries it (Phase 8); override bodies inherit. The Finder's `finder_keys` free fn is migrated from `src/tui/finder.rs::handle_finder_key` (translated to take `KeyBind` + `&mut App`).
  - `Shortcuts<App>` block owns `defaults() -> Bindings<Action>`.
  - Owns `visibility(&self, action, ctx) -> Visibility` and `state(&self, action, ctx) -> ShortcutState` — moves cursor-position-dependent visibility logic out of `App::enter_action` into the affected impls (CiRuns Activate `Hidden` at EOL; Package/Git/Targets Activate `Disabled` when their preconditions fail). The bar **label** is always `Action::bar_label()`.
  - Registers a free dispatcher `fn(Action, &mut App)`.
  - Optionally overrides `bar_slots(ctx)` for paired layouts and data-dependent omission (ProjectList: emits `(Nav, Paired(CollapseRow, ExpandRow, "expand"))` and `(Nav, Paired(ExpandAll, CollapseAll, "all"))`; CiRuns: omits toggle row when no ci data).
  - Overrides `vim_extras` to declare pane-action vim binds (`ProjectListAction::ExpandRow → 'l'`, `CollapseRow → 'h'`).
- Implement `Navigation<App> for AppNavigation` and `Globals<App> for AppGlobalAction`.
- **Add `BarPalette` to `tui_pane::bar` and widen `render_status_bar`.** New public type:
  ```rust
  // tui_pane/src/bar/palette.rs
  pub struct BarPalette {
      pub enabled_key_style:    Style,
      pub enabled_label_style:  Style,
      pub disabled_key_style:   Style,
      pub disabled_label_style: Style,
      pub separator_style:      Style,
  }
  impl Default for BarPalette { /* every field = Style::default() — neutral, no colors */ }
  ```
  Re-export at the crate root: `pub use bar::BarPalette;`. `render_status_bar` becomes `render_status_bar(focused, ctx, keymap, framework, &BarPalette) -> StatusBar`. `support::push_slot` / `push_paired` consume the palette and select between `enabled_*` / `disabled_*` based on `RenderedSlot::state`. `support::SEPARATOR` (`"  "`) styles with `palette.separator_style`. The framework ships **no cargo-port colors** in `Default` — that constructor is theme-neutral; binaries supply their own palette to get any color at all.
- **Cargo-port supplies its palette.** Add `cargo_port_bar_palette() -> BarPalette` (or equivalent constructor on the binary side; placement `src/tui/render.rs` or a new `src/tui/bar_palette.rs`) that wires the existing `ACCENT_COLOR` (yellow + bold) for keys, plain for labels, `SECONDARY_TEXT_COLOR` for disabled keys/labels — exactly matching the pre-refactor look produced by `shortcut_spans`. The binary's render path constructs this once per draw (or holds a `LazyLock` / `OnceCell`) and passes `&palette` into `render_status_bar`.
- **Builder call order.** The keymap builder is a typestate: `register_navigation::<AppNavigation>()` and `register_globals::<AppGlobalAction>()` are reachable only in `Configuring` state (before any `register::<Pane>(...)` call). They each capture the `<N>`/`<G>`-monomorphized renderer fn pointer for the bar (Phase 13's `Keymap::render_navigation_slots` / `render_app_globals_slots`). Concrete order at the binary's startup:
  ```rust
  let keymap = tui_pane::Keymap::<App>::builder()
      .config_path(path)?                // settings phase
      .load_toml(path)?
      .vim_mode(VimMode::Enabled)
      .with_settings(app.settings.clone())
      .on_quit(app::on_quit)
      .on_restart(app::on_restart)
      .dismiss_fallback(app::dismiss_fallback)
      .register_navigation::<AppNavigation>()?
      .register_globals::<AppGlobalAction>()?
      .register::<PackagePane>(PackagePane)
      .register::<GitPane>(GitPane)
      // ... every other Shortcuts<App> impl
      .build_into(&mut app.framework)?;
  ```
  The `register_navigation::<N>` and `register_globals::<G>` calls are required for any non-empty pane set (`finalize` returns `KeymapError::NavigationMissing` / `GlobalsMissing` otherwise). They must precede the first `register::<Pane>(...)`.
- **`impl AppContext for App`** — required for `Framework<App>` to instantiate. Per Phase 6's narrowed surface, only `framework()` and `framework_mut()` need bodies; `set_focus` ships with a default that delegates to `self.framework_mut().set_focused(focus)`. cargo-port takes the default unless a focus-change side-effect (logging, telemetry) becomes useful — decide at impl time.
- Build the app's `Keymap` at startup. Old `App::enter_action` and old `for_status_bar` still exist; the new keymap is populated but not consumed yet.
- **New field name: `framework_keymap: tui_pane::Keymap<App>`** on the `App` struct, distinct from the existing `keymap: Keymap` subsystem field (paths/watcher/`ResolvedKeymap`). The two coexist for Phases 14–18 — the binary's `keymap` subsystem keeps powering `App::enter_action` and `for_status_bar`; `framework_keymap` is populated by tests first and then consumed by Phase 18's structural Output-cancel preflight. Phase 19 collapses or deletes the legacy field once dispatch fully flips through the framework.
- **Old path stays authoritative; no deletes in Phase 14.** Phase 14 lands trait impls + the parallel `framework_keymap` alongside today's input/render code. Do not delete or rewrite the old `App::enter_action`, `for_status_bar`, `shortcut_spans`, `Shortcut`, `InputContext`, `for_status_bar` callers, or any `handle_*_key` handler in Phase 14 — Phase 19 owns those deletions. The only exception is mechanical dead code that compile errors expose (e.g. an unused import after the action enums migrate to `tui_pane::action_enum!`).
- **Implement one pane fully first as the pattern.** Land Package end-to-end — `Pane<App>`, `Shortcuts<App>`, free-fn dispatcher, `visibility`/`state`, plus the focused-Package bar snapshot — and only then replicate to Git / ProjectList / CiRuns / Lints / Targets / Output / Lang / Cpu / Finder. Catches register-order mistakes and trait-bound mismatches once instead of ten times.

**Phase 14 chunking schedule.** Each chunk below is committable standalone (compiles + clippy clean + tests pass) and preserves the Phase 14–18 parallel-path invariant — the old path stays authoritative through the Phase 14 chunks. Phase 19 performs the live dispatch/bar cutover; Phases 20–22 remove the legacy model, overlay, and focus/input compatibility layers that Phase 19 intentionally left in place. This schedule is the canonical implementation order; later chunks may be split or merged as implementation teaches us, but the Package-first principle does not move.

- **14.1 — framework `BarPalette` + widened `render_status_bar` (✅ landed, commit `9c6bb25`).** Theme-neutral `Default`. Binary still uses the old path; tests pass at the boundary.
- **14.2 — Package end-to-end as the pattern (✅ landed).** Single chunk that lands the *minimum* scaffolding the first pane needs:
  1. `AppPaneId` enum with all eventual variants (plain `#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]`, not an `Action` — defining only `Package` now would force a churn-rename on every later chunk).
  2. `framework_keymap: tui_pane::Keymap<App>` field on `App` + `impl AppContext for App` returning `&self.framework` / `&mut self.framework` (default `set_focus` body).
  3. `NavigationAction` and `AppGlobalAction` enum definitions via `tui_pane::action_enum!` with one static `bar_label` per variant; `impl Navigation<App> for AppNavigation` (six directional consts + `defaults` + `dispatcher`); `impl Globals<App> for AppGlobalAction` (`render_order` + `defaults` + `dispatcher`). Required by the keymap builder typestate before any `register::<Pane>(...)` call — even with one pane, the builder's `finalize` returns `KeymapError::NavigationMissing` / `GlobalsMissing` without these.
  4. Migrate **only `PackageAction`** from `src/keymap.rs`'s local `action_enum!` to `tui_pane::action_enum!` with one static `bar_label` per variant. Every other enum stays on the local 2-positional macro untouched. (Historical note: 14.2 shipped the migration in 3-positional form; Phase 14.3.5 later re-migrated `PackageAction` to the 2-positional form once the macro grew the optional shorthand.)
  5. `impl Pane<App> + Shortcuts<App>` for `PackagePane`: `APP_PANE_ID = AppPaneId::Package`, `SCOPE_NAME = "package"`, `defaults()` mirroring today's bindings, `visibility` / `state` for the cursor-position rules, plus a no-op free-fn dispatcher through Phase 18. Behavior stays identical because the legacy path remains authoritative; Phase 19 wires real framework-keymap dispatch. `Mode::Navigable` default — no override.
  6. Build the `framework_keymap` at startup: `tui_pane::Keymap::<App>::builder().register_navigation::<AppNavigation>()?.register_globals::<AppGlobalAction>()?.register::<PackagePane>(PackagePane).build_into(&mut app.framework)?`. No `with_context` yet (so no `anyhow` dep). Old `App::enter_action` and `for_status_bar` untouched.
  7. One snapshot test: focused-Package bar through `render_status_bar(&FocusedPane::App(AppPaneId::Package), &app, &framework_keymap, app.framework(), &BarPalette::default())` against the new static-label form. No `cargo_port_bar_palette()` yet — the snapshot locks the new framework-rendered span sequence, not pre-refactor color.
  8. Mandatory non-snapshot test that fits this chunk: Package `pane.state(Activate, ctx)` returns `ShortcutState::Disabled` when no actionable row exists, and `ShortcutState::Enabled` on the `CratesIo` row when `crates_version` is known.
  - **Excluded from 14.2:** other panes' enum migrations, other panes' trait impls, `cargo_port_bar_palette()`, `anyhow`, splitting `tui_pane::GlobalAction` from the binary's `GlobalAction`, deleting the local `action_enum!` macro, mechanical fixes to old-path call sites that didn't break compile. All of these come later.

  ### 14.2 Retrospective

  **What worked:**
  - Single new file `src/tui/framework_keymap.rs` housed every new framework-side type (`AppPaneId`, `NavigationAction`, `AppGlobalAction`, `AppNavigation`, `PackagePane`, `build_framework_keymap`) plus the `impl AppContext for App`; only a small set of pre-existing files needed edits (`src/keymap.rs`, `src/tui/mod.rs`, `src/tui/app/mod.rs`, `src/tui/app/construct.rs`, and test registration). Parallel-path invariant held — 802/802 tests pass with the legacy keymap untouched.
  - Inherent-method facade on `PackageAction` (`pub const ALL: &'static [Self] = <Self as tui_pane::Action>::ALL;` plus forwarders for `toml_key` / `description` / `from_toml_key`) let every legacy call site (`src/keymap.rs`, `src/tui/keymap_ui.rs`, `src/tui/shortcuts.rs`) compile unchanged. This pattern will repeat verbatim for `GitAction` (14.3) and the six panes in 14.4 — captured as a per-pane checklist item below.

  **What deviated from the plan:**
  - The plan named the rule "`Activate` `Disabled` on `CratesIo` field without a version." The legacy `package_fields_from_data` *omits* the `CratesIo` row entirely when `crates_version` is `None`, so that exact condition can never fire. The semantically equivalent rule that actually shipped: `Activate` is `Enabled` only when the cursor is on a row whose dispatch has a real effect (today, only `CratesIo`); every other row — and packages without a `crates_version` — return `Disabled`. The mandatory test renamed accordingly (`package_activate_state_disabled_when_no_crates_version`).
  - `impl Globals<App>` landed on the action enum itself (`impl Globals<App> for AppGlobalAction`) per the plan wording; `impl Navigation<App>` landed on a separate ZST (`AppNavigation`) per the plan wording. The asymmetry is deliberate — the trait surface allows either — but worth flagging so 14.7's binary-side `tui_pane::GlobalAction` split doesn't accidentally try to make them match.
  - `App` gained two new fields, not one: `framework: tui_pane::Framework<App>` is required by `AppContext` and was implicit in the plan ("returning `&self.framework`"). The Phase 11 retrospective at line 2129 already named this — Phase 14.2 just wired it.
  - `framework_keymap` is constructed by first storing a placeholder empty `Keymap` in the struct literal and then overwriting it via `framework_keymap::build_for_app(&mut app)`. Two-step because `build_into(&mut app.framework)` needs a `&mut` borrow that can't overlap the struct-literal expression. Clean enough — keep this pattern in 14.3+.
  - Per-pane modules carry a single `#![allow(dead_code, reason = "Phase 14.x landing; later chunks construct these")]` at the file head. Saves per-variant attributes; remove during the Phase 19 wiring pass.

  **Surprises:**
  - `tui_pane::action_enum!` generates only a trait impl, no inherent impl, so call sites that take `Type::method` as a fn pointer (e.g. `PackageAction::toml_key` passed into `write_scope`) need either an inherent forwarder or UFCS rewriting. The forwarder approach is invisible to call sites and adds ~6 lines per migrated enum — far cheaper than touching every call site twice (once for the migration, once for the Phase 19 cleanup).
  - Building a real `App` for the snapshot test required reaching the existing `make_app` fixture in `src/tui/app/tests/mod.rs`, so the new tests live at `src/tui/app/tests/framework_keymap.rs` (registered alongside `panes.rs`, `state.rs`, etc.) rather than under `src/tui/framework_keymap.rs::tests`. The framework_keymap module's own `#[cfg(test)] mod tests` keeps lightweight non-`App` tests (variant counts, action trait facade).
  - `cargo build` after the migration was clean on the first try — the tests also went green except for the one rule-mismatch case noted above. The framework's typestate builder, `BarPalette::default()`, and `render_status_bar` all worked from outside the framework crate exactly as the Phase 13 / 14.1 retrospectives predicted.

  **Implications for remaining phases:**
  - **14.3 + 14.4 + 14.5 + 14.6 each migrate their pane's existing local `action_enum!` invocation in `src/keymap.rs`** — each migration must add the same inherent-method facade (`ALL` / `toml_key` / `description` / `from_toml_key`) so legacy call sites compile. This is mechanical; bake it into the per-chunk checklist.
  - **The `framework_keymap.rs` module is the single home for `Pane<App>` / `Shortcuts<App>` impls.** Each subsequent chunk extends it with one more `XPane` ZST + `impl Pane<App> for XPane` + `impl Shortcuts<App> for XPane` and one more `.register::<XPane>(XPane)` call in `build_framework_keymap`. The dispatcher fns stay no-op through 14.7 — the legacy path remains authoritative.
  - **The "state(Activate) Disabled when no actionable target" pattern from 14.2 generalizes to every pane that conditionally dispatches.** CiRuns' `visibility(Activate) → Hidden` (planned for 14.4) and Targets' `Activate` / `ReleaseBuild` (also 14.4) reuse the same function signature: `fn(&self, Self::Actions, &App) -> ShortcutState` (or `… -> Visibility`), reading `panes.<name>.content()` plus `viewport.pos()` and returning the typed enum. No `&mut App` needed in any of these.
  - **Old path stays authoritative — confirmed by 802 passing tests.** The framework keymap is built but never consulted at dispatch time; the bar still renders through `for_status_bar`. That separation lets every subsequent chunk land additively without touching live behavior, and the Phase 19 swap becomes one wiring change at the input/render boundary.
  - **No `AppNavigation` / `Globals<App>` rewiring needed in 14.3–14.6.** The single registration in `build_framework_keymap` is enough — adding panes only appends `.register::<P>(P)` calls. 14.7 will revisit `Globals<App>` to expand `AppGlobalAction` from one variant (`Find`) to the full app-extension set (`Find` + `OpenEditor` + `OpenTerminal` + `Rescan`).
  - **Per-pane snapshots in 14.3–14.6 assert on `bar.pane_action` only.** Do not assert on `bar.nav` or `bar.global` from a per-pane snapshot. The nav and global regions have separate ownership (the framework keymap's `Navigation` / `Globals` singletons) and separate test coverage in 14.7 (after `AppGlobalAction` grows to four variants) and Phase 23 (regression rebinding tests). Per-pane snapshots that lock the global region today would silently bind every pane's snapshot to `AppGlobalAction = { Find }`, forcing seven re-blessings the moment 14.7 lands. 14.2's `focused_package_bar_renders_pane_action_labels` is the canonical pattern: read `bar.pane_action`, ignore the rest.

  ### 14.2 Review

  Minor findings (applied without prompting):
  - **14.3 / 14.4 / 14.5 / 14.6 chunk text** — explicit "add inherent-method facade" deliverable folded into each chunk, mirroring 14.2's `PackageAction` pattern; visibility convention pinned to `pub`.
  - **14.4 / 14.6 chunk text** — pre-flight bullet added: verify the planned `visibility` / `state` rule against the legacy upstream row builder before naming the rule (14.2 surfaced one rule whose precondition was already filtered out by `package_fields_from_data`).
  - **14.9 closeout** — the file-head `#![allow(dead_code, reason = ...)]` on `src/tui/framework_keymap.rs` stays through Phase 18 and is removed by Phase 19; per-variant attributes in 14.3–14.6 are not needed.
  - **Phase 19/22 cleanup list** — `framework_keymap.rs` file-head `dead_code` allow and the temporary `build_for_app` startup shim are Phase 19 cleanup; the per-pane inherent-method facades delete in Phase 22 with the remaining input compatibility layer.
  - **Task list** — pre-14.2 umbrella tasks `#181`–`#189` deleted in favor of the per-chunk schedule.

  Significant findings (walked through `/adhoc_review`, applied):
  - **Find 2 → Phase 14.7** rewritten as three scoped jobs: (1) grow `AppGlobalAction` from `{ Find }` to `{ Find, OpenEditor, OpenTerminal, Rescan }`, (2) path-qualify any new `tui_pane::GlobalAction` references, (3) delete only the local `macro_rules! action_enum` block. Legacy `crate::keymap::GlobalAction` enum stays alive through Phase 19 (parallel-path invariant).
  - **Find 3 → Phase 14.8** amended to also collapse 14.2's two-step `framework_keymap` initializer: `App::new` chain becomes `Result<App, anyhow::Error>`, the placeholder + `process::abort` shims delete, the temporary `build_for_app` shim deletes, the framework keymap is built once via a single `?`-returning chain.
  - **Find 8 → option B applied:** per-pane snapshots in 14.3–14.6 assert on `bar.pane_action` only. 14.7 lands dedicated `bar.global` (and optional `bar.nav`) snapshots once `AppGlobalAction` is final. Avoids re-blessing seven files.
  - **Find 10 → Phase 16** owns the `App::set_focus` override: writes both `framework.set_focused(focus)` and (for app-pane focuses) the legacy `self.focus.set(id.to_legacy())` mirror. Phase 22 deletes only the legacy mirror line; the override itself survives because Phase 23's funnel test depends on it.

- **14.3 — second pane (Git) replicating 14.2's pattern (✅ landed).** Migrate `GitAction` to `tui_pane::action_enum!` (3-positional, `pub` visibility) **and add the inherent-method facade** (`pub const ALL` + `pub fn toml_key` / `description` / `from_toml_key` forwarders to the `tui_pane::Action` trait impl, mirroring `PackageAction` from 14.2) so legacy `src/keymap.rs` / `src/tui/keymap_ui.rs` / `src/tui/shortcuts.rs` call sites compile unchanged. Add `GitPane` `Pane<App>` + `Shortcuts<App>` impls in `src/tui/framework_keymap.rs`; register after Package in the builder chain; one Git bar snapshot. Validates the pattern with one extra pane before scaling to seven more. Surfaces any pattern bug once, here, instead of nine times across 14.4–14.6.

  ### 14.3 Retrospective

  **What worked:**
  - `GitAction` migration was a 1:1 copy of 14.2's `PackageAction` recipe — flip the macro from local 2-positional to `tui_pane::action_enum!` 3-positional, append the four-method inherent facade verbatim. No call site touched in `src/keymap.rs::write_scope` / `vim_mode_conflicts` / `resolve_pane_scopes`, `src/tui/keymap_ui.rs`, or `src/tui/panes/actions.rs::handle_detail_key`. The pattern transfers cleanly with zero unexpected breakage.
  - `GitPane` lives entirely inside `src/tui/framework_keymap.rs`. Adding it was three appendages: the `GitPane` ZST, `impl Pane<App> + Shortcuts<App> for GitPane`, and one `.register::<GitPane>(GitPane)` call inserted after Package in `build_framework_keymap`. The `build_for_app` shim / two-step initializer in `src/tui/app/construct.rs` is untouched — Phase 14.8 still owns that cleanup.
  - The `state(Activate)` rule for Git generalizes from Package's pattern with one substitution. Where Package reads `package_fields_from_data(pkg)` + cursor pos against `DetailField::CratesIo`, Git reads `git_row_at(git, pos)` against `GitRow::Remote(r)` with `r.full_url.is_some()`. The signature is identical — `fn(ctx: &App) -> ShortcutState`, no `&mut`, no extra ctx surface needed. The pattern that 14.2's retrospective predicted ("generalizes to every pane that conditionally dispatches") held.

  **What deviated from the plan:**
  - The plan asked for "one Git bar snapshot." Three tests shipped: the snapshot plus two `state(Activate)` tests (Disabled on a flat field row, Enabled on a `Remote` row whose `full_url` is `Some`). The state tests are the Git analogue of 14.2's mandatory Package state tests — and the test_additions_are_minor feedback rule treats test scaffolding as auto-apply. Total Git test count: 3 (one bar snapshot + two state tests).
  - Workspace test count moved 802 → 805, all green; 14.2's retrospective number was the just-pre-14.3 baseline.

  **Surprises:**
  - `git_fields_from_data` always emits `RateLimitCore` and `RateLimitGraphQl` regardless of data state, so a default `GitData::default()` has 2 flat rows even when nothing else exists. The first `Remote` row therefore sits at `pos == 2`, not 0. The Enabled test pins this offset explicitly.
  - `cargo mend --workspace` surfaced 25 warnings + 2 errors in the pre-cleanup tree. The hard errors were 14.2's nested bridge wrapper (`pub(crate) mod bridge`, `pub(crate) fn build_for_app`); the cleanup replaced that wrapper with a module-level `pub(super) build_for_app` shim and narrowed the `PackageAction` / `GitAction` facades to `pub(crate)`, leaving the current mend baseline clean. Phase 22 deletes both facades; Phase 14.8 still deletes the temporary `build_for_app` shim.

  **Implications for remaining phases:**
  - **The pattern is stable.** 14.4 can scale the Git recipe to ProjectList / CiRuns / Lints / Targets / Lang / Cpu without further surprises. Each pane is: enum migration in `src/keymap.rs` + facade lines, ZST + trait impls + state rules in `src/tui/framework_keymap.rs`, one register call, one snapshot test. Pre-flight verification (per the plan's 14.4 bullet) against the legacy upstream row builder is the only judgment call per pane.
  - **`bar_label` mid-state literal duplication is now addressed in 14.3.5.** `GitAction::Activate => ("activate", "activate", ...)` repeats the toml_key. Counted ~20 such redundant invocations queued across 14.4–14.7 (Lints, CiRuns, Targets, ProjectList, Output, Finder, AppGlobalAction). The macro grammar tweak that defaults `bar_label` to the toml_key has been pulled forward to 14.3.5 (per the 14.3 review block below) so 14.4–14.7 ship the cleaner 2-positional form.

  ### 14.3 Review

  Minor findings (applied without prompting):
  - **14.4 chunk text** — pre-flight bullet rewritten: removed reference to non-existent `panes/ci.rs::ci_runs_rows`; pointed at the actual cursor→entry surfaces for each pane (CiRuns: `viewport.pos()` vs `ci_runs.len()`; Lints: `lint.viewport.pos()` indexed into `runs` slice; Targets: `build_target_list_from_data`; ProjectList: `src/tui/project_list/mod.rs`). Added the unconditional-vs-conditional row check (Find 9) so the `GitData` rate-limit-rows surprise doesn't recur.
  - **14.6 Finder pre-flight** — pinned the candidate field path (`app.overlays.finder.is_open()` vs `app.finder`); requires confirmation against the live `App::overlays` struct before writing the override.
  - **14.7 chunk text** — added a one-sentence callback explaining why the `bar.global` snapshot lands here (per Find 8 in 14.2 review): locking it during 14.3–14.6 would force seven re-blessings when `AppGlobalAction` grows from 1 to 4 variants.
  - **Phase 19 deletion list** — tightened the `build_for_app` shim bullet from "if 14.8 already deleted it, this bullet is moot" to "already removed by 14.8; verify gone." 14.8 owns the deletion outright.
  - **Phase 23 funnel test** — added note that `AppContext::set_focus is the single funnel` assertion runs after the Phase 22 focus cleanup, not post-Phase-16. Phase 16 introduces the override; the test only passes once Phase 19 swaps dispatch through it and Phase 22 removes direct focus writers.

  Significant findings (walked through `/adhoc_review`, applied):
  - **Find 3 → Phase 14.4** per-pane state-rule budget: `state()` is for "slot rendered but disabled," `visibility()` is for "slot not rendered." Default every 14.4 pane to `ShortcutState::Enabled` unless legacy code proves otherwise. Confirmed cases: Lints / Lang / Cpu / CiRuns are all `Enabled` (CiRuns' EOL behavior belongs in `visibility`); Targets requires verification of `TargetEntry::kind` against legacy dispatch; ProjectList's heavy work is paired-rows + vim-extras, not state.
  - **Find 4 → Phase 14.4** split into 14.4a / 14.4b / 14.4c by ascending weight: 14.4a = Targets + Lints + Lang + Cpu (trivial, four commits), 14.4b = CiRuns (medium — visibility EOL rule), 14.4c = ProjectList (heavyweight — paired rows, two new variants, vim_extras). Order is reversed from the architect's suggestion at the user's direction so the trivial pattern stabilizes first.
  - **Find 5 → New Phase 14.3.5** pulls the `action_enum!` macro tweak forward: optional 2-positional grammar where `bar_label` defaults to the toml_key. Saves ~20 redundant `("foo", "foo", ...)` writes across 14.4–14.7. `PackageAction` and `GitAction` migrate to the 2-positional form in the same commit. Existing 3-positional fixtures stay valid; the explicit form remains available when bar_label genuinely differs.

- **14.3.5 — `action_enum!` macro grammar tweak (✅ landed).** One commit, framework-side. In `tui_pane/src/keymap/action_enum.rs`, accept either form:
  - 2-positional: `Variant => ("toml_key", "description")` — `bar_label` defaults to the `toml_key` literal.
  - 3-positional: `Variant => ("toml_key", "bar_label", "description")` — explicit override when the bar label genuinely differs from the toml key.

  Migrate `PackageAction` (in `src/keymap.rs`) and `GitAction` (same file) to the 2-positional form in the same commit so neither retains needless duplication. The framework's existing 3-positional fixtures in `tui_pane/tests/macro_use.rs` stay valid (they exercise the explicit form). Add one fixture exercising the 2-positional form so both paths have test coverage.

  **Pre-flight:** run `cargo mend --workspace` and `cargo build` after the macro change; if any third existing call site of `tui_pane::action_enum!` exists outside the binary, decide whether to migrate it now (free) or pin it to the explicit form (zero churn). The known call sites at this point are the binary's two enums (`PackageAction`, `GitAction`) plus the framework's own fixtures — both small.

  **Why this lands here, not Phase 19:** deferring means writing the duplicate `("foo", "foo", ...)` literal ~20 more times across 14.4–14.7, then editing each line again at Phase 19 cleanup. ~30 min now saves ~40 line-edits across the next four chunks.

  ### 14.3.5 Retrospective

  **What worked:**
  - The two-arm `macro_rules!` design landed in one edit. The 2-positional arm expands directly into the 3-positional arm by passing `$toml_key` twice (`( $toml_key , $toml_key , $desc )`), so the implementation is one extra arm with zero new codegen. Both arms coexist; explicit 3-positional invocations like `CrossCrateNavAction` (`up => ("up", "up", "Move up")`) keep matching the 3-positional arm without ambiguity.
  - Migration of `PackageAction` and `GitAction` was mechanical: drop the second `"activate"` / `"clean"` literal per variant. No call sites in `src/keymap.rs::write_scope` / `vim_mode_conflicts` / `resolve_pane_scopes`, no Shortcuts impls, no facade methods needed adjustment — the inherent-method facade still forwards through `tui_pane::Action`, and the `Action` impl produces identical `bar_label()` output (`"activate"` / `"clean"`) because the 2-positional arm defaults `bar_label` to the toml key.
  - Two new fixtures lock both paths: in-crate `FooShort` (in `tui_pane/src/keymap/action_enum.rs::tests`) and cross-crate `CrossCrateShortAction` (in `tui_pane/tests/macro_use.rs`). The cross-crate fixture is the one that catches `$crate::*` resolution problems through the macro arm forwarding — an in-crate test cannot exercise that path.

  **What deviated from the plan:**
  - Plan said "macro lives in `tui_pane/src/action.rs`." Actual path is `tui_pane/src/keymap/action_enum.rs` (it's part of the keymap module surface). Plan text updated inline.
  - Plan asked for "one fixture exercising the 2-positional form." Two shipped (one per crate boundary). The cross-crate fixture is necessary because the in-crate fixture cannot exercise the `$crate::action_enum!` self-recursion path that the 2-positional arm uses to forward into the 3-positional arm — a regression in `$crate` resolution would only show up across crate boundaries.

  **Surprises:**
  - `cargo mend` baseline is unchanged at 25 warnings + 2 errors. The 4 `pub`-visibility warnings on each of `PackageAction`'s and `GitAction`'s inherent facades are still flagged (Phase 22 deletes those facades; no new mend findings on the facade surface). The macro-arm forwarding produces zero additional mend findings.
  - Workspace test count moved 805 → 807 (one in-crate fixture + one cross-crate fixture added). All green.

  **Implications for remaining phases:**
  - **14.4–14.7 ship 2-positional by default.** Plan text already says this (per the Find 5 amendment). Six new enums queued for 14.4 (Targets / Lints / Lang / Cpu / CiRuns / ProjectList) and three for 14.5–14.7 (Output / Finder / `AppGlobalAction` expansion) all use the 2-positional form unless a genuine `bar_label != toml_key` case appears — the recipe text already says "use 3-positional only when the bar label genuinely differs from the toml key."
  - **No new dependency or sequencing constraint.** 14.3.5 is purely additive: the 3-positional form continues to compile and work. `NavigationAction` and `AppGlobalAction` in `src/tui/framework_keymap.rs` were left in 3-positional form because their bar labels happen to equal their toml keys but the migration is optional churn — leave them on 3-positional through Phase 19 unless a future chunk has reason to touch them.
  - **`NavigationAction` is a permanent 3-positional case.** No remaining 14.x chunk edits its body, and Phase 19's deletion list does not touch `framework_keymap.rs` enum bodies. The 3-positional form is correct (bar_label happens to equal toml_key for all six directional variants) and stays untouched through Phase 19.

  ### 14.3.5 Review

  Minor findings (applied without prompting):
  - **14.2 chunk text** — historical note appended: "14.2 shipped the migration in 3-positional form; Phase 14.3.5 later re-migrated `PackageAction` to the 2-positional form once the macro grew the optional shorthand." Keeps the chunk body accurate for future readers.
  - **14.3.5 retrospective implications** — appended `NavigationAction` permanent 3-positional note: no remaining chunk edits its body, no Phase 19 deletion-list normalization needed.
  - **Phase 19 deletion list** — added explicit "no `action_enum!` form-normalization sweep" rule: any 3-positional invocation surviving past 14.7 is correct (either bar_label genuinely differs, or coincides and converting is pure churn).
  - **14.5 Output pre-flight** — added per-variant bar_label/toml_key check; `OutputAction::Cancel` named as the canonical risk (toml_key `"cancel"`, bar may show `"esc"` or `"close"`).
  - **Phase 15 framework-internal note** — added one-line clarification: framework-internal action enums (Toasts, Settings, Keymap, hand-rolled `GlobalAction`) are not subject to the 14.3.5 form choice; new variants follow the same rule, but Phase 15 adds none.
  - **Phase 24 `ToastsAction::Activate`** — added inline comment explaining why the example uses 3-positional form (`bar_label "open"` ≠ `toml_key "activate"`, per 14.3.5).

  Significant findings (walked through `/adhoc_review`, applied):
  - **Find 2 + 12 → Phase 14.7** `AppGlobalAction` expansion form decision: flip the existing `Find` row from 3-positional to 2-positional and add the three new rows (`OpenEditor`, `OpenTerminal`, `Rescan`) in 2-positional form. All four variants have bar_label == toml_key. The 14.7 chunk text now pins this explicitly and references the 14.4 per-variant pre-flight.
  - **Find 6 → 14.4 per-pane recipe** added a per-variant bar_label/toml_key verification pre-flight: before defaulting any variant to 2-positional, read the legacy bar-render path for that pane and confirm the displayed label equals the toml_key; any variant where they differ uses 3-positional for that variant only. Catches the `OutputAction::Cancel`-class regression where 2-positional would silently change the bar label. The recipe text at 14.4 now governs 14.5–14.7 too via the existing back-reference.

- **14.4 — replicate to remaining `Mode::Navigable` panes, easiest first (✅ landed).** Three sub-chunks ordered by weight ascending so the trivial register-and-snapshot pattern stabilizes before the heavier work lands:
  - **14.4a — Targets, Lints, Lang, Cpu** (trivial batch — one commit per pane). Each pane is ZST + 5-line `Pane` impl + `Shortcuts` impl with `defaults()` mirroring today's bindings + `state` returning `Enabled` unconditionally (per Find 3 above) + register call + `bar.pane_action` snapshot. **Targets exception:** verify whether `TargetEntry::kind` actually drives a `state()` rule in legacy `handle_target_action` — if so, ship that rule with Targets; otherwise default to `Enabled`. Expected: ~40 lines per pane, four commits.
  - **14.4b — CiRuns** (medium). Same recipe as 14.4a plus the EOL `visibility(Activate) → Hidden` override (rule: `position >= ci_runs.len()` returns `Hidden`). The mandatory `pane.visibility(Activate, ctx) → Hidden at EOL` test ships in this commit.
  - **14.4c — ProjectList** (heavyweight). Two new action variants (`ExpandRow` / `CollapseRow`), paired-row `bar_slots` override emitting `(Nav, Paired(CollapseRow, ExpandRow, "expand"))` and `(Nav, Paired(ExpandAll, CollapseAll, "all"))`, `vim_extras` declarations (`ExpandRow → 'l'`, `CollapseRow → 'h'`), plus the snapshot. Lands last so the ZST + register pattern is fully proven on six panes before the override surfaces wire in.

  **Per-pane recipe (applies to every sub-chunk):** enum migration to `tui_pane::action_enum!` (2-positional per 14.3.5; use 3-positional only when the bar label genuinely differs from the toml key — **per-variant verification pre-flight:** before defaulting any variant to 2-positional, read the legacy bar-render path for that pane and confirm the displayed label string equals the toml_key for that variant; any variant where they differ uses 3-positional for that variant only — `OutputAction::Cancel` is the canonical example, where `toml_key = "cancel"` may map to a bar label like `"esc"` or `"close"`) **plus inherent-method facade** (per 14.3), trait impls in `src/tui/framework_keymap.rs`, registration after the prior pane in the builder chain, `bar.pane_action` snapshot. Each commit: enum migration to `tui_pane::action_enum!` **plus inherent-method facade** (per 14.3), trait impls in `src/tui/framework_keymap.rs`, registration, per-pane snapshot. **Pre-flight per pane:** before naming any `visibility` / `state` rule, walk the legacy row producer for that pane (concrete sources: `src/tui/panes/support.rs::build_target_list_from_data` for Targets; for CiRuns, `ctx.panes.ci.viewport.pos()` against `ctx.panes.ci.content().map(|d| d.ci_runs.len())` — there is no `ci_runs_rows` helper, the cursor maps directly into the runs slice; for Lints, `ctx.lint.content().map(|d| d.runs.as_slice())` indexed by `ctx.lint.viewport.pos()`; ProjectList builds rows in `src/tui/project_list/mod.rs`; Lang and Cpu have no row-conditional Activate logic in the legacy path). Identify (a) which rows the producer emits *unconditionally* (e.g. `git_fields_from_data` always emits the two `RateLimit*` rows even on a default `GitData`, so the first remote sits at `pos == 2` not 0 — same trap likely lurks in CiRuns headers and Lang's static rows) vs (b) which rows are gated on data presence; pin the test cursor offset against (a) explicitly. Phase 14.2 surfaced one rule (`Activate Disabled on CratesIo without version`) whose precondition was already filtered out by `package_fields_from_data`, mooting the rule as named — the pre-flight prevents that recurring. **Per-pane state-rule budget. Use `state()` only when legacy behavior has a real row/action precondition.** Package and Git already do (see 14.2 / 14.3 retrospectives). Default every other pane to `ShortcutState::Enabled` unless reading the legacy code proves otherwise. Concretely: Lints `Activate` works on any non-empty runs row, no precondition. Lang and Cpu have no `Activate` dispatch in `src/tui/panes/actions.rs` at all. CiRuns `Activate` is gated by EOL — that belongs in `visibility()` (return `Hidden`), not `state()`. ProjectList `Clean` always works. Targets `Activate` has actionable per-row payload (`TargetEntry::kind`) and may warrant a `state()` rule — verify against `build_target_list_from_data` first. ProjectList's heavy work is paired rows + vim extras (the heavyweight commit), not disabled-state logic. **Distinction to preserve:** `Visibility::Hidden` = don't render the slot; `ShortcutState::Disabled` = render the slot grayed because the action is currently inert; `Enabled` = normal — don't overcomplicate it. ProjectList additionally adds `ExpandRow` / `CollapseRow` variants and the paired-row `bar_slots` override; CiRuns additionally adds the EOL `visibility(Activate) → Hidden` override (rule: position `>= ci_runs.len()` returns `Hidden`; mandatory test ships here).
### 14.4 Retrospective

**What worked:**
- ZST + `Pane` + `Shortcuts` impl pattern from 14.3 replicated cleanly across six panes (`ProjectListPane`, `LangPane`, `CpuPane`, `TargetsPane`, `LintsPane`, `CiRunsPane`); each pane is ~30–60 lines in `src/tui/framework_keymap.rs` plus its enum migration block in `src/keymap.rs`.
- Per-variant 3-positional `tui_pane::action_enum!` audit caught all the cases where the legacy bar label diverges from the toml_key (`TargetsAction::Activate` "run", `TargetsAction::ReleaseBuild` "release", `CiRunsAction::Activate` "open", `CiRunsAction::FetchMore` "fetch more", `CiRunsAction::ToggleView` "branch/all", `CiRunsAction::ClearCache` "clear cache", `LintsAction::Activate` "open", `LintsAction::ClearHistory` "clear cache", `ProjectListAction::ExpandAll` "+", `ProjectListAction::CollapseAll` "-", `ProjectListAction::ExpandRow` "→", `ProjectListAction::CollapseRow` "←").
- `CiRunsPane::visibility(Activate) → Hidden` at EOL plus its mandatory test landed exactly as the plan called for, with no extra state-rule scope.
- All 617 workspace tests pass (was 609 + 8 new — six per-pane bar snapshots, two `CiRuns` `Activate` visibility tests).

**What deviated from the plan:**
- The per-pane snapshots and `CiRuns` visibility test landed alongside the impl in a single edit pass rather than one commit per pane. The pattern was already proven by 14.3, so the per-pane commit cadence the plan suggested ("Expected: ~40 lines per pane, four commits") added churn without de-risking anything.
- `ProjectListAction` migration and the new `ExpandRow` / `CollapseRow` variants forced a touch on the legacy `ResolvedKeymap::defaults` (binding both to `Shift+Right` / `Shift+Left` so the existing `defaults_scope_map_consistency` test stays green) plus the golden TOML at `tests/assets/default-keymap.toml`. The plan's "framework-only" framing did not anticipate that the shared enum needs a default in the legacy path too.
- The `ProjectListPane::bar_slots` override emits `BarSlot::Paired(...)` per the plan, but the framework's `runtime_scope.rs::render_bar_slots` reduces every `Paired(a, _, _)` to `Single(a)` via `.primary()` — the second action and the `"expand"` / `"all"` separator are dropped at the bar layer. This was already pinned by `tui_pane`'s existing `render_bar_slots_uses_paired_primary_for_lookup` test; the test in `src/tui/app/tests/framework_keymap.rs::focused_project_list_bar_renders_pane_action_and_nav_slots` was scoped to assert the primary actions' rendered output, with a comment naming Phase 17's paired-slot rendering prerequisite as the work that must land before Phase 23's `+/-` and `←/→ expand` rebind tests.
- `LangPane` and `CpuPane` got their own minimal action enums (`LangAction`, `CpuAction`, each with a single `Clean` variant) defined directly in `src/tui/framework_keymap.rs`. The plan said to "mirror today's bindings"; today's Lang routes through `PackageAction` and Cpu has `PaneId::Cpu => {}` (no dispatch). One-variant local enums are the cleanest way to give each pane its own `SCOPE_NAME` without dragging Package's actions into a Lang/Cpu TOML scope. No facade is needed — these enums have zero callers outside the framework path.
- `ProjectListAction::ExpandRow | CollapseRow` got a no-op match arm in `src/tui/input.rs::handle_normal_key` because the legacy `handle_normal_key` reads `KeyCode::Right` / `KeyCode::Left` *before* it consults the keymap, so the dispatch never reaches the match for those keys — but the match needs to compile for all variants. Phase 16 wires the framework path through here.

**Surprises:**
- `tui_pane::action_enum!`'s 2-positional form is invocation-wide, not per-variant: a single `tui_pane::action_enum!` block can't mix 2-positional and 3-positional rows. Once one variant in a block needs 3-positional, every variant in that block has to spell it out (e.g. `Clean => ("clean", "clean", "Clean project")`). Worth pinning explicitly in 14.5 / 14.6 / 14.7 so they don't trip on it.
- The `tui_pane::KeyBind` constructor `From<char>` is not a `const fn`, so static initialization of `vim_extras` arrays uses the public `code` / `mods` field literals (`KeyBind { code: KeyCode::Char('l'), mods: KeyModifiers::NONE }`) rather than `KeyBind::from('l')`. Workable but verbose — a `const fn KeyBind::from_char(c: char) -> Self` would clean every future `vim_extras` declaration up.
- `ci_runs_activate_visibility` reads `ctx.ci.content()`, not `ctx.panes.ci.content()` — the per-pane recipe pre-flight in 14.4 named the wrong field path (`ctx.panes.ci.viewport.pos()` and `ctx.panes.ci.content()`). Cargo-port's `Ci` lives directly on `App` as `app.ci`, not under `App::panes`. Same trap caught the `Lint` field (`app.lint`, not `app.panes.lint`).

**Implications for remaining phases:**
- **Phase 14.5 (Output):** the 2-positional-is-invocation-wide rule means `OutputAction` is 3-positional iff *any* variant's bar label differs from its toml_key. Per the existing 14.5 pre-flight, `Cancel` is the named risk; once any variant needs 3-positional, all do.
- **Phase 14.7:** the `expand_row` / `collapse_row` rows added to the default keymap golden file at `tests/assets/default-keymap.toml` round-trip cleanly because `Shift+Right` / `Shift+Left` are not navigation-reserved (the loader's `is_navigation_reserved` check fires only when modifiers are NONE). Phase 16's deletion of `NAVIGATION_RESERVED` removes that constraint; until then, the framework-path defaults stay on `KeyCode::Right` / `KeyCode::Left` (no modifier) so the runtime dispatch matches today's Right/Left arrow behavior.
- **Phase 23 (regression tests):** the `+/-` and `←/→ expand` rebind tests need proper paired rendering for pane-emitted `BarSlot::Paired` slots. Today `runtime_scope.rs::render_bar_slots` discards the secondary action and separator. The paired-rendering extension must land before those rebind snapshots can assert against `←/→` and `+/-` strings; the alternative is to weaken the assertions to single-key checks, which loses the test's value. Flag this as a Phase 23 prerequisite, not a Phase 14 oversight.
- **Phase 22 cleanup list:** four new inherent facades (`TargetsAction`, `CiRunsAction`, `LintsAction`, `ProjectListAction`) joined the existing `PackageAction` / `GitAction` facades; Phase 22's "delete the inherent-method facade blocks" item names them all. `LangAction` / `CpuAction` are framework-only enums with no facade — no Phase 22 work for them.
- **Phase 16 (`set_focus` override):** unaffected. The new pane registrations don't touch focus mutation; the override's current placement is fine.

### 14.4 Review

Minor findings (applied without prompting):
- **Phase 22 cleanup list (facade exclusion)** — added explicit "`LangAction` / `CpuAction` have no facade; Phase 22 has no work for those two enums" note alongside the existing facade-deletion bullet.
- **Phase 16 (`Left`/`Right` reroute)** — named the `ProjectListAction::ExpandRow | CollapseRow => {}` no-op match arm in `src/tui/input.rs::handle_normal_key` as the seam Phase 16 fills, so the deletion of the top-of-`handle_normal_key` `KeyCode::Right` / `KeyCode::Left` arms moves their bodies into the existing match arm rather than re-introducing a new one.
- **14.5 / 14.6 form-rule pin** — appended "form is invocation-wide" note to both chunks: `tui_pane::action_enum!` does not mix 2-positional and 3-positional rows; one diverging variant forces every variant in the block to spell its bar_label. Phase 14.5 also gained a one-line clarification that `OutputAction::Cancel`'s `toml_key` stays `"cancel"` regardless of what `bar_label` resolves to (Phase 18's reverse-lookup is keyed on `toml_key`, not `bar_label`).
- **14.9 closeout test list** — annotated the mandatory-test list with ✅ landed markers for the items that already shipped (Package state in 14.2, focused-Package snapshot in 14.2, CiRuns Activate-EOL visibility in 14.4b) and re-scoped the "deferred follow-up" list from "nine remaining snapshots" to "Output and Finder only" (eight per-pane snapshots already shipped in 14.3 / 14.4).
- **14.9 closeout polish** — added optional `pub const fn KeyBind::from_char(c: char) -> Self` to `tui_pane::keymap::key_bind` as a one-line readability fix for the `framework_keymap.rs::PROJECT_LIST_VIM_EXTRAS` static array; ProjectList is the only consumer today (Output/Finder add no vim extras), so this is purely polish, not a correctness fix.

Significant findings (walked through `/adhoc_review`, applied with explicit user approval):
- **Find 1 + 11 → Phase 17 prerequisite** — folded the paired-slot rendering prerequisite into the bundled `tui_pane` cutover-prerequisites phase. It lands one of three concrete options (extend `RenderedSlot` with optional secondary triple; add a parallel `render_bar_slot_spans`; or introduce a `RenderedSlot::Paired` variant) before Phase 23 rebind snapshots assert against the `+/-` and `←/→ expand` rows. The framework extension is purely additive and does not block on Phase 19.
- **Find 15 → Phase 20 keymap-model cleanup** — added a one-bullet item: re-bless `tests/assets/default-keymap.toml` against the framework's defaults so `expand_row` / `collapse_row` flip from `Shift+Right` / `Shift+Left` (the legacy `ResolvedKeymap` defaults chosen in 14.4c so the round-trip passes the `is_navigation_reserved` check) to plain `Right` / `Left` once the Phase 20 keymap model cleanup retires `ResolvedKeymap`. The asset itself survives; only its contents change.

Findings 5, 6, 8, 9, 12, 14 came back as "no action — already correct or unaffected" and got no plan edits.

- **14.5 — Output (`Mode::Static` override) (✅ landed).** `OutputAction` enum definition (2-positional per 14.3.5; use 3-positional only if the bar label differs from the toml key) + inherent facade per 14.3 + `OutputPane` impls in `src/tui/framework_keymap.rs`. `Pane::mode()` returns `Mode::Static`. Per-pane snapshot. **Pre-flight (per-variant bar_label check):** `OutputAction::Cancel` is the canonical risk — today's Output bar may show `"esc"` or `"close"` rather than `"cancel"`. Read the legacy bar-rendering path (`src/tui/render.rs` and `src/tui/output.rs`) and confirm what the bar actually displays for each variant; ship 3-positional for any variant whose displayed label diverges from its toml key. **Form is invocation-wide (per 14.4 retrospective):** `tui_pane::action_enum!` does not mix 2-positional and 3-positional rows in one block — once any variant needs 3-positional, every variant in the same block has to spell its bar label out explicitly (e.g. `Clean => ("clean", "clean", "Clean project")`). The reverse-lookup predicate `is_key_bound_to_toml_key(OutputPane::APP_PANE_ID, OutputAction::Cancel.toml_key(), bind)` in Phase 18's structural-Esc preflight is keyed on `toml_key`, not `bar_label`, so the toml key stays `"cancel"` regardless of what the displayed label resolves to.
- **14.6 — Finder (`Mode::TextInput` override) + `finder_keys` migration (✅ landed).** `FinderAction` enum definition (2-positional per 14.3.5; 3-positional only where bar_label differs from toml_key — and per 14.4 retrospective, the form is invocation-wide: any one variant needing 3-positional forces every variant in the block to spell its bar_label out) + inherent facade per 14.3 + `FinderPane` impls in `src/tui/framework_keymap.rs`; `Pane::mode()` returns `Mode::TextInput(finder_keys)` while open, `Mode::Navigable` otherwise; migrate today's `handle_finder_key` from `src/tui/finder.rs` to the `finder_keys(KeyBind, &mut App)` free fn referenced from `Pane::mode()`. **Pre-flight:** confirm exactly which `App` field carries the open/closed flag the legacy `handle_finder_key` gates on (most likely `app.overlays.finder.is_open()`, but verify against the live `App::overlays` struct — Phase 14.2's retrospective flagged that `app.framework` was added to `App` without explicit plan call-out because `impl AppContext for App` requires it; same risk here for `app.overlays.finder` vs `app.finder`). Pin the field path in the implementation comment. Per-pane snapshot. Mandatory non-snapshot tests ship here: `Pane::mode()` arms (open vs closed); `'k'` typed in the search box inserts `'k'` even with vim mode on (handler is sole authority).

### 14.5 + 14.6 Retrospective

**What worked:**
- `OutputAction` and `FinderAction` migrations dropped in beside the existing 14.4 enums in `src/keymap.rs` with no surprises — the per-variant bar_label/toml_key pre-flight caught the divergences immediately (`Cancel` → `"close"` for both panes, `Activate` → `"go to"` for `FinderAction`), so both enums shipped 3-positional from the first commit.
- `OutputPane`'s `Mode::Static` override and `FinderPane`'s `Mode::TextInput(finder_keys)` override both came in as ~10 lines each. The framework's `fn mode() -> fn(&Ctx) -> Mode<Ctx>` signature was the right call — letting the closure read `app.overlays.is_finder_open()` directly avoided having to bolt new state onto `FinderPane`.
- The `finder_keys(KeyBind, &mut App)` free fn delegates straight to legacy `super::finder::handle_finder_key(app, bind.code)` — single-line body, zero behavior change. The mandatory `'k'`-into-query test exercises the framework's `Mode::TextInput` payload directly via the `fn` pointer (no full dispatch wiring needed), which is exactly testable against shipped code.

**What deviated from the plan:**
- **No inherent facade for `OutputAction` / `FinderAction`.** The plan listed both in the facade group, but neither has a legacy `ResolvedKeymap` consumer (no `output:` / `finder:` `ScopeMap` field on `ResolvedKeymap`). Adding the facade triggered `dead_code` warnings; per the project's hard rule on `#[allow]` autonomy, the principled fix was to drop the facades entirely rather than suppress. Phase 18's reverse-lookup snippet `OutputAction::Cancel.toml_key()` resolves through the `tui_pane::Action` trait at the call site (the trait is already in scope in `framework_keymap.rs`), so the inherent forwarder added nothing. **Implication for Phase 22:** the facade-removal list should drop `OutputAction` / `FinderAction` — there is no facade to remove.
- **Finder `bar_slots` not overridden.** The plan implied `FinderAction::Activate` / `Cancel` would render via the default `bar_slots()`; that's what landed (no override). Open Finder hides every region anyway via `Mode::TextInput`; closed Finder is never actually focused in normal flow.

**Surprises:**
- `Mode::TextInput` suppresses **every** bar region (Nav, PaneAction, Global), not just Nav. This means open-finder bar tests assert all three regions are empty — confirmed against `tui_pane::bar::global_region.rs:31-32`, `pane_action_region.rs:24-25`, and `nav_region.rs:33`. Worth pinning in the Phase 21 overlay-routing plan and Phase 23 regression suite: when Finder is open and the user hits a global like `Ctrl+r` (Rescan), today's legacy path still fires it; once Phase 21 routes overlays through the framework, `Mode::TextInput` will swallow that key. The plan needs to spell out which globals (if any) should remain reachable while finder is open.
- `tui_pane::KeyBind` provides `From<char>` and `From<KeyCode>` but no inherent `plain` constructor. `KeyBind::from('k')` was the path of least resistance for the test, vs the legacy `crate::keymap::KeyBind::plain` style. No code change needed — just a vocabulary note for 14.7's `AppGlobalAction` defaults if any test wants to construct a `tui_pane::KeyBind` directly.
- The framework `finder_keys` handler can call into the legacy `handle_finder_key` body without lifetime / borrow grief because `Mode::TextInput`'s payload is `fn(KeyBind, &mut Ctx)` — a plain function pointer, not a closure that captures the framework keymap. The migration to "real" framework dispatch in Phase 21 won't require restructuring this handler.

**Implications for remaining phases:**
- **Phase 19 — facade deletion list.** The bullet that lists facades to remove should not include `OutputAction` / `FinderAction`. Update inline.
- **Phase 23 — globals reachable in `Mode::TextInput`.** Today, while finder is open, the legacy global dispatch in `src/tui/input.rs` short-circuits before `handle_global_key`. The framework's `Mode::TextInput` matches that behavior exactly (every region suppressed). Phase 23 must pin the policy: keep globals suppressed while finder/settings are editing, OR allow a small allow-list (e.g. `Quit`, `Restart`). The plan currently says nothing about this. Adding a Phase 23 sub-bullet.
- **Phase 14.7 — local `action_enum!` macro deletion.** Two more enums (`OutputAction`, `FinderAction`) now use `tui_pane::action_enum!`. The local-macro-deletion job at 14.7 step 3 covers them implicitly (it deletes the macro once every binary action enum has migrated), but the plan listed only the 14.2–14.6 enums explicitly. Worth listing `OutputAction` / `FinderAction` alongside the others so the Phase 14.7 implementer doesn't miss a usage.

### 14.5 + 14.6 Review

Plan amendments folded in from a Plan-subagent review of phases 14.7 onward. Twelve findings; seven minor applied directly, two significant approved & applied (Finding 3 + Finding 11 merged into one Phase 23 bullet), one significant approved & applied (Finding 2), two confirmed no-action (Findings 5 + 10).

- **14.7 step 3 rewritten to verification-only** (Find 2). The local `macro_rules! action_enum` block does not delete at 14.7 — `crate::keymap::GlobalAction` still uses it and survives the parallel-path invariant through Phase 19. Step 3 is now a checklist confirming all eight pane / app-extension enums migrated. The macro deletes in Phase 20 with `GlobalAction`. Phase 20 keymap-model cleanup grew a corresponding bullet.
- **Phase 19 facade list** (Find 1). Dropped `OutputAction` and `FinderAction` from the inherent-method facade deletion bullet — neither has a facade (no legacy `ResolvedKeymap` consumer); `Excluded:` clause grew to name them alongside `LangAction` / `CpuAction`.
- **Phase 18 reverse-lookup snippet** (Find 4). Added a "trait import at the call site" note: `OutputAction::Cancel.toml_key()` resolves through the `tui_pane::Action` trait now that the facade is gone — `use tui_pane::Action;` must be in scope at the `src/tui/input.rs` snippet site.
- **Phase 15 `Mode::TextInput` payload constraint** (Find 6). Added a one-line rule: `Mode::TextInput(handler)` payload is `fn(KeyBind, &mut Ctx)` — handlers cannot capture state, must be free fn or static method. Auxiliary state routes through `&mut Ctx`. Applies to `settings_edit_keys` / `keymap_capture_keys` / `finder_keys`.
- **Phase 19 "do not delete `handle_finder_key`"** (Find 7). Added an explicit no-deletion note: the legacy `src/tui/finder.rs::handle_finder_key` body survives Phase 19 because `framework_keymap.rs::finder_keys` calls it. Only the legacy *short-circuit path* through `input.rs::handle_normal_key` deletes.
- **Phase 23 `Mode::TextInput` framework-side bar test** (Find 8). Added a `tui_pane/tests/bar_rendering.rs` test asserting `Mode::TextInput` suppresses every region against a `MockApp`, independent of which app pane is focused. Pins the rule the cargo-port-side `focused_finder_open_bar_suppresses_all_regions` test covers for Finder only.
- **Phase 23 globals-suppression policy + Ctrl+r acceptance test** (Find 3 + Find 11, merged). Pinned the policy: no globals fire while focused pane is `Mode::TextInput`. Bit-for-bit parity with today's legacy short-circuit. No allow-list this phase — any opt-in for `Quit` / `Restart` / `Rescan` is post-Phase-19 framework-API design. Named acceptance test: rebind `AppGlobalAction::Rescan` to its default `Ctrl+r`, focus open Finder, assert the rescan dispatcher does *not* fire.
- **Phase 19 / Phase 23 `KeyBind` API note** (Find 9). Documented `KeyBind::from('k')` / `KeyBind::from(KeyCode::Tab)` as canonical (no `plain` constructor on `tui_pane::KeyBind`); struct-literal form is canonical for `static` initializers since `From<char>` is not `const`.
- **Phase 14.9 mandatory-test list** (Find 12). Flipped `(Pending 14.6.)` markers to `✅ Landed in 14.6.` for the two Finder tests; updated the deferred-snapshot count from "two remaining" to "ten landed across 14.3–14.6, deferral list cleared."

Findings 5 (form-rule already correctly applied to 14.5/14.6) and 10 (the paired-slot prerequisite unaffected by 14.5/14.6) came back as no-action confirmations.

- **14.7 — `AppGlobalAction` expansion + retire local `action_enum!` macro.** (✅ landed) Three scoped jobs:
  1. Expand `AppGlobalAction` to `{ Find, OpenEditor, OpenTerminal, Rescan }` with the existing default bindings (`'/'` / `'e'` / `'t'` / `Ctrl+r`). These are the app-owned globals; the framework's own globals (Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss) live in `tui_pane::GlobalAction`. **Form:** flip the existing `Find` row from 3-positional to 2-positional and add the three new rows in 2-positional form — all four variants have bar_label == toml_key, so the canonical post-14.3.5 form applies. Confirm bar_label==toml_key for each new variant against the legacy bar before defaulting to 2-positional (per the per-pane recipe pre-flight in 14.4).
  2. Path-qualify any new framework-global reference as `tui_pane::GlobalAction` so it's clearly separate from the legacy binary `crate::keymap::GlobalAction` at any new call site reading through the framework keymap.
  3. **Verify the migration is complete** — confirm these binary action enums all use `tui_pane::action_enum!`: `PackageAction`, `GitAction`, `TargetsAction`, `CiRunsAction`, `LintsAction`, `ProjectListAction`, `OutputAction`, `FinderAction`. The legacy `crate::keymap::GlobalAction` is the **only** remaining user of the local `macro_rules! action_enum` block at `src/keymap.rs:216-250`. **Do not delete the local macro at 14.7** — `GlobalAction` survives through Phase 19 (parallel-path invariant: `ResolvedKeymap.global` field stays alive until then). The local macro deletes in Phase 20 alongside `GlobalAction` itself; this step at 14.7 is verification-only.

  The legacy `crate::keymap::GlobalAction` *enum* and `ResolvedKeymap.global` field **stay alive through Phase 19** (parallel-path invariant). Phase 14 adds the new framework path beside the old path; Phase 20 deletes the legacy enum and field after the Phase 19 cutover proves the framework path live.

  14.7 also lands the global-region and nav-region bar snapshots that 14.3–14.6 deliberately deferred (per Find 8 in the 14.2 review block above): one snapshot exercising `bar.global` against the four-variant `AppGlobalAction` and (optionally) one snapshot exercising `bar.nav` to lock the framework's pane-cycle row + nav-region rendering. Reason for the deferral: locking the global region in a per-pane snapshot during 14.3–14.6 would bind every pane's snapshot to `AppGlobalAction = { Find }`, forcing seven re-blessings the moment the four-variant set lands here. Per-pane snapshots from 14.3–14.6 read `bar.pane_action` only and do not need re-blessing.

  ### 14.7 Retrospective

  **What worked:**
  - Three jobs landed in one tight edit: enum expansion + defaults + verification + two snapshot tests. `cargo build` / 823 nextest tests / clippy / install all clean.

  **What deviated from the plan:**
  - Plan said "all four variants have bar_label == toml_key, so the canonical post-14.3.5 form applies" and instructed flipping `Find` to 2-positional. The pre-flight pre-flight (read the legacy bar before defaulting to 2-positional) caught a discrepancy: legacy `src/tui/shortcuts.rs:170-203` shows `OpenEditor` as `"editor"` and `OpenTerminal` as `"terminal"` — bar_label != toml_key for two of the four. Since `tui_pane::action_enum!`'s 2-positional form is invocation-wide, the entire block stays 3-positional. `Find` did **not** flip. Final form: `Find => ("find", "find", …); OpenEditor => ("open_editor", "editor", …); OpenTerminal => ("open_terminal", "terminal", …); Rescan => ("rescan", "rescan", …)`.
  - Job 2 ("path-qualify any new framework-global reference as `tui_pane::GlobalAction`") was a no-op in 14.7: dispatchers stay no-op through Phase 18, and no new call sites in `src/` reach `tui_pane::GlobalAction`. The directive remains correct for whichever later phase first reads through the framework keymap's globals scope.

  **Surprises:**
  - The plan's per-variant pre-flight (added in the 14.2 Review block under Find 6) caught a real form regression at 14.7 — the plan author's "all four are bar_label==toml_key" assertion was wrong, and following it blindly would have silently relabeled the global strip from `editor`/`terminal` to `open_editor`/`open_terminal` once Phase 19 wires the framework path through. The pre-flight is doing its job.
  - `bindings!` accepts heterogeneous expressions on the LHS — chars (`'/'`, `'e'`, `'t'`) coexist with `KeyBind::ctrl('r')` in the same table. No type juggling needed.

  **Implications for remaining phases:**
  - **Phase 19** (legacy deletion): when `crate::keymap::GlobalAction::OpenEditor`/`OpenTerminal` delete and the bar reads through `AppGlobalAction` instead, the per-variant labels `editor`/`terminal` survive unchanged because of the 3-positional bar_labels just landed. **The global strip itself does change at cutover, however** — the framework walks `tui_pane::GlobalAction` first then `AppGlobalAction`, so the rendered order becomes `quit, restart, keymap, settings, dismiss, find, editor, terminal, rescan` (today's order is `find, editor, terminal, settings, keymap, rescan, quit, restart`); `dismiss` newly appears in the strip; `next`/`prev` move out of the global strip into the nav-region pane-cycle row. Any pre-Phase-19 snapshot that locks `bar.global` ordering will re-bless at the cutover commit. The four `AppGlobalAction` labels themselves do not re-bless.
  - **Phase 23** (regression rebinding tests): `bar.global` is now snapshot-locked to four `AppGlobalAction` labels (`find`, `editor`, `terminal`, `rescan`) by `focused_package_bar_renders_four_app_globals`. The test uses `contains` (inclusion-only), which survives the cutover ordering shift in Phase 19; Phase 23's full ordered snapshot will lock the post-cutover order. Any future `AppGlobalAction` variant addition updates the 14.7 inclusion test plus the Phase 23 ordered snapshot — that is the intended cost of locking the strip.

  ### 14.7 Review

  Plan amendments folded in from a Plan-subagent review of phases 14.8 onward. 14 findings; 7 minor applied directly, 4 significant approved & applied, 3 confirmed no-action.

  - **Find 1 → Phase 19** new **Wire dispatchers** sub-section enumerating every dispatcher fn that flips from no-op to a real body (framework `GlobalAction`, `AppGlobalAction`, `AppNavigation`, all ten app pane dispatchers). Phrased as "route to the existing/extracted operation body" — no commitment to specific helper names. **Atomic cutover constraint** added: dispatcher bodies and legacy-handler deletions land in the same Phase 19 changeset.
  - **Find 5 → Phase 19/20 split** deletion list grew three bullets: `handle_global_key` fn body + 11-arm match + call site is Phase 19 cutover work; every `app.keymap.current().global` accessor read site and `ResolvedKeymap.global: ScopeMap<GlobalAction>` move with the keymap-model cleanup in Phase 20. References phrased by symbol/path, not line numbers.
  - **Find 12 → Phase 19** main-loop paragraph at `src/tui/terminal.rs` swap got a **Land together** sentence: main-loop poll-site swap and `tui_pane::GlobalAction::Quit`/`Restart` dispatcher bodies must land in the same changeset (write/read pair).
  - **Find 13 → Phase 18** reverse-lookup snippet `app.keymap` → `app.framework_keymap`. Surrounding paragraph updated to name `tui_pane::Keymap`'s public surface.
  - **Find 2 + 4 (merged) → 14.7 retrospective** Implications-for-remaining-phases bullet widened: per-variant labels `editor`/`terminal` survive Phase 19 cutover unchanged, but the global strip's order and membership do change (framework-first walk; `dismiss` newly visible; `next`/`prev` move to nav-region). Pre-Phase-18 ordered-`bar.global` snapshots will re-bless at cutover.
  - **Find 3 + 11 (merged) → Phase 23** new lead bullet: snapshot-truth ordering for `bar.global` under default bindings (`quit, restart, keymap, settings, dismiss, find, editor, terminal, rescan`) plus the prerequisite that `AppGlobalAction::dispatcher()` body has landed in Phase 19 so the side-effect counter is observable.
  - **Find 6 → Phase 21** overlay cleanup got an audit-pass note naming `src/tui/input.rs::handle_overlay_editor_key` as a concrete call site that reads `GlobalAction::OpenEditor` outside `handle_global_key` and must flip when overlay input no longer intercepts before normal global dispatch.
  - **Find 10 → Phase 23** dispatch-parity bullet: markdown-quoting artifact corrected — `OpenEditor` rebind target is `'E'`, not the literal backtick the broken markdown rendered.
  - **Findings 7, 8, 9** confirmed no-action (Phase 16 `set_focus` override unaffected; no remaining phase still assumes 2-positional `AppGlobalAction`; no remaining phase still assumes the local `action_enum!` macro deletes at 14.7).

- **14.8 — `cargo_port_bar_palette()` + `anyhow` introduction + collapse 14.2's two-step initializer.** (✅ landed) Add `anyhow = "1"` to the binary's root `Cargo.toml` `[dependencies]` *at the same edit* that introduces the first `with_context` call. The first such call is the framework-keymap build wrapper, which also drives the initializer cleanup:
  1. Make `App::new` (and the `construct::AppBuilder` chain) return `Result<App, anyhow::Error>`. Delete the two `process::abort` shims in `src/tui/app/construct.rs:222-227`.
  2. Replace 14.2's placeholder-then-overwrite pattern with a single `?`-returning chain: `tui_pane::Keymap::<App>::builder().load_toml(keymap_path).with_context(|| format!("loading keymap from {}", display_path))?.register_navigation::<AppNavigation>()?.register_globals::<AppGlobalAction>()?.register::<PackagePane>(PackagePane).register::<…>(…)…build_into(&mut app.framework).with_context(|| "building framework keymap")?`. The framework keymap is built once, not twice.
  3. Delete the temporary `build_for_app` shim in `src/tui/framework_keymap.rs` (added in 14.2 as a temporary forwarding fn). `construct.rs` calls `build_framework_keymap(&mut app.framework)?` directly, or `build_framework_keymap` becomes the single entry point and `construct.rs` calls it through the public name.
  4. Add `cargo_port_bar_palette()` and route the binary's draw call through `render_status_bar` with `&cargo_port_bar_palette()`. Old `for_status_bar` call site lives until Phase 19.

  ### 14.8 Retrospective

  **What worked:**
  - The single ?-returning chain reads cleanly. `App::new`'s signature change to `Result<App, anyhow::Error>` propagated through three call sites (`tui/terminal.rs`, `tui/interaction.rs::tests::make_app`, `tui/app/tests/mod.rs::make_app_with_config`) — the binary entry point surfaces the error via `tracing::error!` + `ExitCode::FAILURE`; the two test helpers use `.expect("App::new must succeed in tests")`.
  - `cargo_port_bar_palette()` lives in `src/tui/render.rs` next to the legacy `render_status_bar` call site. The framework call's result is bound to `_framework_bar` and dropped — Phase 19 swaps the legacy `shortcut_spans` flatten path for the framework regions.
  - Build/test/clippy/install all green: 625 tests pass, `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean, `cargo install --path .` replaced the binary on first try.

  **What deviated from the plan:**
  - **`load_toml(keymap_path)` was not added to the framework-keymap build chain.** The plan's example chain has `.load_toml(keymap_path).with_context(...)?` as the first step. Adding it now breaks startup whenever a user keymap TOML exists with a populated `[global]` table: `tui_pane`'s `apply_toml_overlay::<AppGlobalAction>` runs over the same `[global]` table that `apply_toml_overlay::<tui_pane::GlobalAction>` already consumes (line 604 in `tui_pane/src/keymap/builder.rs`), and unknown action keys (the framework's `quit`/`restart`/etc. seen by the `AppGlobalAction` overlay, or the app's `find`/`open_editor`/etc. seen by the framework's `GlobalAction` overlay) raise `KeymapError::UnknownAction` (line 467). The framework currently lacks shared-`[global]`-table coordination between two registered globals enums; supplying that is a `tui_pane` change beyond Phase 14.8's scope. The framework keymap therefore stays on defaults until the coordination lands; Phase 18 wires framework TOML loading, while the legacy `crate::keymap::ResolvedKeymap` path keeps owning broad dispatch until Phase 19.
  - **`with_context` placement narrowed to one call.** Without `load_toml`, only the `build_into` step earns a `with_context(|| "building framework keymap")?`. The plan's example showed two `with_context` wrappers; the chain shipped one.
  - **Not a deviation but worth noting:** the test-mod `#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, reason = "tests should panic on unexpected values")]` block was added to `src/tui/interaction.rs::tests` to match the established pattern at `src/tui/app/mod.rs:103-112` and `src/tui/framework_keymap.rs:667-672`. The `App::new` Result return forces the test helper to fall through this allow.

  **Surprises:**
  - The framework's `apply_toml_overlay` does not coordinate two registered globals enums against the shared `[global]` TOML table. `validate_toml_scopes` accepts `[global]` (line 639), but the per-overlay action validation in `apply_toml_overlay` (line 466) hard-rejects keys not in the enum being parsed. Two `Globals` enums splitting the table cleanly is an unstated requirement; today's code rejects either side's foreign keys.
  - `dead_code` on `App::framework_keymap` (the field warned during 14.2–14.7's parallel-path) cleared the moment the field was read by `tui_pane::render_status_bar` from the binary's draw call — no separate `#[allow(dead_code)]` needed for the field itself.

  **Implications for remaining phases:**
  - **Phase 18 / 18 must explicitly handle TOML loading for the framework keymap.** Either: (a) add a `tui_pane` API that splits the shared `[global]` table across multiple registered globals enums (each overlay consumes only the keys mapping to its own enum, leaves the rest); or (b) delete the legacy `[global]` schema and split the user TOML into `[framework_global]` + `[app_global]` (or similar) at cutover. Option (a) preserves user TOML compatibility; option (b) is a one-shot migration. The cutover commit decides.
  - **The `with_context(|| "building framework keymap")?` call is the only context wrapper today.** Phase 18 / 18's TOML-load wiring will introduce the second `with_context` (around `load_toml`) when it solves the shared-table coordination problem.
  - **`framework_keymap::build_framework_keymap` is now `pub(super)` (was `pub(crate)`).** It's only called from `construct.rs`, a sibling under `tui::app`. Phase 19 may need to widen visibility again if dispatcher wiring code lives in another crate-level module — straightforward edit if so.
  - **The legacy `for_status_bar` + `shortcut_spans` + `shortcut_display_width` glue stays alive through Phase 19.** Phase 19's deletion list already names them (line 2993). The framework `_framework_bar` call site enters Phase 19's swap as the new draw path; it loses the leading underscore and the legacy left/center/right rendering blocks delete around it.

  ### 14.8 Review

  Plan amendments folded in from a Plan-subagent review of phases 14.9 onward. 15 findings; 6 minor applied directly, 4 significant approved & applied, 2 significant approved as no-op (the Phase 18 redirect on Finding 1 + option (a) on Finding 2 collapsed the cross-phase prerequisite chain), 3 confirmed no-action.

  - **Find 1 (redirected) → Phase 18** new **Load TOML into the framework keymap** bullet, owner because Phase 18's structural-Esc preflight is the first user-facing read of `app.framework_keymap` for resolved action lookup. Bullet enumerates: call `.load_toml(keymap_path)` before `register_navigation` / `register_globals` / pane registrations; wrap with `with_context(|| format!("loading keymap from {}", display_path))`; depend on Phase 17; add a test asserting a user TOML rebind reaches the framework path. **Invariant** stated explicitly: before any framework-keymap path becomes authoritative or user-visible, the framework keymap must load the same `keymap::keymap_path()` TOML the legacy `ResolvedKeymap` consumed.
  - **Find 2 → Phase 17** new sub-section between Phase 18 and Phase 19 — `tui_pane` shared-`[global]`-table coordination. **Decision: option (a)** (per-key skip-when-peer-registered). Behavior contract, implementation sketch, and four tests (existing combined `[global]` TOML loads cleanly, framework-owned rebind applies, app-owned rebind applies, truly unknown key still errors).
  - **Find 3 → no-op** (option (a) preserves the `## Risks and unknowns` "no breaking change" claim; no edit needed there).
  - **Find 4 → Phase 23** new lead bullet at the head of the bar-on-rebind block: hard acceptance prerequisite that Phase 18 + Phase 17 must have shipped before any "Rebinding X to Y" test runs.
  - **Find 5 (minor, applied) → Phase 19** main-loop "Land together" paragraph augmented to clarify the post-14.8 `match App::new(...) { Err => ExitCode::FAILURE, Ok(app) => app }` envelope is unaffected by Phase 19.
  - **Find 6 → no-op** (Finding 1's Phase 18 redirect placed the prerequisite intra-phase; the test's load dependency is now local and visible inline).
  - **Find 7 (minor, applied) → Phase 19** new **Visibility note** paragraph: re-widen `framework_keymap::build_framework_keymap` from `pub(super)` if any new caller lands outside `tui::app`.
  - **Find 8 (minor, applied) → Phase 19** main-loop paragraph clarifies that 14.8's `match App::new` block at `src/tui/terminal.rs:165-180` survives the cutover.
  - **Find 9 → no-op** (Finding 1's Phase 18 redirect plus Finding 2's Phase 17 placement removed the case for absorbing TOML wiring into 14.9; closeout stays small).
  - **Find 10 (minor, applied) → Phase 17** paired-slot rendering prerequisite moved into the bundled `tui_pane` PR so Phase 23 snapshots inherit it.
  - **Find 11 (minor, applied) → Phase 25** new TOML-load surface paragraph noting `[toasts]` reuses whatever loader pattern Phase 17 picks.
  - **Find 14 (minor, applied) → Phase 23** new test-mod allow pattern anchor at the top of the section, naming the canonical `#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, reason = "tests should panic on unexpected values")]` block.
  - **Findings 12, 13, 15** confirmed no-action (no later mention of "framework keymap is built once not twice" needs editing; `cargo_port_bar_palette()` survival in Phase 19 is already correct; no `app.framework_keymap` read site is invalidated by 14.8).
- **14.9 — closeout (✅ landed).** Any mandatory tests not yet landed by their owning chunk. The single file-head `#![allow(dead_code, reason = "...")]` on `src/tui/framework_keymap.rs` (added in 14.2) stays through Phase 18 and is removed by Phase 19's wiring swap, when every variant becomes constructed; do **not** add per-variant `#[allow(dead_code)]` at registration time in 14.3–14.6 — the file-wide allow already covers them. **Optional polish:** add a `pub const fn KeyBind::from_char(c: char) -> Self` to `tui_pane::keymap::key_bind` so static `vim_extras` arrays can drop the `KeyBind { code: KeyCode::Char('l'), mods: KeyModifiers::NONE }` literal in `framework_keymap.rs::PROJECT_LIST_VIM_EXTRAS`. ProjectList is the only consumer today; Finder/Output add no vim extras, so this is purely a readability win, not a correctness fix. `cargo build && cargo +nightly fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings` clean across the workspace.

### 14.9 Retrospective

**What worked:**
- Mandatory test verification was a no-op walk: every item on the test list was already marked ✅ landed in its owning chunk (14.2 / 14.4b / 14.6), and the per-pane snapshot deferral list was already cleared in the 14.5+14.6 review block. No tests to write.
- The optional `KeyBind::from_char` polish landed cleanly: one `pub const fn` on the existing `impl KeyBind` block in `tui_pane/src/keymap/key_bind.rs`, and the two `PROJECT_LIST_VIM_EXTRAS` rows in `src/tui/framework_keymap.rs` collapsed from a six-line struct-literal each to a single-line `KeyBind::from_char('l')` / `KeyBind::from_char('h')` call.
- Workspace check clean on first try: 823 tests pass, clippy `-D warnings` clean, `cargo install --path .` replaced the binary.

**What deviated from the plan:**
- File-head `#![allow(dead_code, ...)]` on `src/tui/framework_keymap.rs` was already in place from 14.2; 14.9 did not need to touch it. Removal still happens at Phase 19 as planned.

**Surprises:**
- The `crossterm::event::KeyCode` / `KeyModifiers` qualifications elsewhere in `framework_keymap.rs` (the seven `From<KeyCode>` impls at lines 146–151, 199, 254, 383, 411, etc.) keep `crossterm` reachable, so removing the two struct-literal `KeyBind { code: …, mods: KeyModifiers::NONE }` rows did not eliminate any imports.

**Implications for remaining phases:**
- None. 14.9 was scoped to closeout polish and produced no new constraints. Phase 17 / 18 / 19 plans stand unchanged.

The Mandatory Phase 14 tests list (CiRuns visibility, Package state, Finder mode, Finder typing, focused-Package snapshot) is **distributed** across the chunks above — each test ships in the chunk that lands its subject. Per-pane snapshots ship with the pane's own chunk; no separate "snapshots follow-up" commit is needed, which moots the Deferred-tests block.

**`anyhow` lands in the binary only when the startup builder wiring actually uses it.** The first call site that benefits from context wrapping is `Keymap::<App>::builder(...).load_toml(path).build_into(&mut framework)?` → wrap with `.with_context(|| format!("loading keymap from {}", path.display()))`. Add `anyhow = "1"` to the root `Cargo.toml` `[dependencies]` **at the same edit** that introduces the first `with_context` call — not earlier as a speculative dep bump. The library (`tui_pane`) does not depend on `anyhow` — only typed `KeymapError` / `KeyParseError` / etc. cross the framework boundary, and the binary adds context at the boundary.

**Phase 14 test priority — compileability first.**

The first binary pass prioritizes (a) the registration chain compiling end-to-end, (b) the focused trait-method tests, and (c) one snapshot for the pattern pane (Package). Per-pane snapshots for the remaining nine come once the registration chain is stable and the pattern is known to work — adding all ten at once before the pattern lands tends to lock in any mistake ten times.

Mandatory Phase 14 tests:

- CiRuns `pane.visibility(Activate, ctx)` returns `Visibility::Hidden` when the viewport cursor is at EOL (hides the slot). ✅ Landed in 14.4b.
- Package `pane.state(Activate, ctx)` returns `ShortcutState::Disabled` when no actionable row exists and `ShortcutState::Enabled` on the `CratesIo` row when `crates_version` is known. ✅ Landed in 14.2.
- Finder `Pane::mode()(ctx)` returns `Mode::TextInput(finder_keys)` while open, `Mode::Navigable` otherwise. ✅ Landed in 14.6.
- Finder migration: typing `'k'` in the search box inserts `'k'` into the query (handler is sole authority — vim keybinds in other scopes do not fire). ✅ Landed in 14.6.
- One pattern snapshot: focused-Package bar under default bindings with `BarPalette::default()` in 14.2; the later Phase 14 palette/parity work uses `cargo_port_bar_palette()`. ✅ Landed in 14.2.

Deferred to a follow-up commit inside Phase 14 (after the registration chain is stable):

- ✅ Ten per-pane bar snapshots landed across 14.3 / 14.4 / 14.5 / 14.6 (Git / Targets / Lints / Lang / Cpu / CiRuns / ProjectList plus the in-Nav-region ProjectList paired-slot check; Output's `Mode::Static` close-label snapshot in 14.5; Finder's open- and closed-mode snapshots in 14.6). The pre-Phase-14.4 deferral list ("nine remaining" snapshots) is fully cleared.

**`build_into` preflight for tests that go through `framework.focused_pane_mode(ctx)`.** Phase 10 made `focused_pane_mode` read from `mode_queries`, which is populated only by `KeymapBuilder::build_into(&mut framework)`. Tests in Phases 13, 14, and 19 that assert on `focused_pane_mode` (bar snapshots, finder mode override, etc.) **must** build the keymap with `build_into`, never `build()` — the latter leaves `mode_queries` empty and silently returns `None` for every `FocusedPane::App(_)` arm. Tests that exercise `Pane::mode()` directly (the trait associated function, no `Framework`) can use `build()` because they don't touch the registry.

### Phase 15 — Reroute overlay input handlers (✅ partially landed; full cutover deferred to Phase 19)

Convert overlay handlers to scope dispatch:

- The Finder's TextInput handler is the free fn `finder_keys(KeyBind, &mut App)` referenced from `Pane<App>::mode()`'s `Mode::TextInput(finder_keys)` return. While the Finder is focused and its mode is `TextInput`, the framework dispatch routes every keystroke to that handler — globals/nav scopes do not fire (the handler is sole authority). The handler dispatches Finder action keys (`Confirm`, `Cancel`) through `keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, ctx)` — `KeyOutcome::Consumed` means a Finder action fired and consumed the keystroke; `KeyOutcome::Unhandled` falls through to the literal `Char(c)` / `Backspace` / `Delete` text-input behavior. (Pre-Phase-9-reset drafts read action enum values via a typed accessor; that accessor was dropped — dispatch-and-observe replaces it.) **`Mode::TextInput(handler)` payload constraint:** the payload type is `fn(KeyBind, &mut Ctx)` — a plain fn pointer, not a closure. Handlers cannot capture state. Finder can use that path because Finder is app-owned; SettingsPane and KeymapPane now use framework-owned command APIs.
- Framework `SettingsPane::handle_key(&mut self, &bind) -> KeyOutcome` replaces today's browse-mode `handle_settings_key` / `handle_settings_adjust_key` dispatch. Browse/Editing modes route through internal mode flag. The dispatch caller checks the return: `KeyOutcome::Consumed` halts; `KeyOutcome::Unhandled` falls through to globals/dismiss. **Phase 15 production transitions** wire the `Browse → Editing` step (Enter/Space on a row) and Save/Cancel returns to `Browse`; Phase 25 later moves edit-buffer mutation into `SettingsPane::handle_text_input`.
- Framework `KeymapPane::handle_key(&mut self, &bind) -> KeyOutcome` replaces browse-mode `handle_keymap_key`. Browse/Awaiting/Conflict modes route through internal mode flag. Same `KeyOutcome` return contract. **Phase 15 production transitions** wire `Browse → Awaiting` (Enter on a row), `Awaiting → Conflict` (captured key collides) or `Awaiting → Browse` (clean rebind), and `Conflict → Browse` (resolve). Phase 28 moves capture mutation into `KeymapPane::handle_capture_key(...) -> KeymapCaptureCommand`. The `#[allow(dead_code, reason = "Phase 15 transitions...")]` on the `EditState` variants in both panes comes off in this phase.
- **Toasts focus gate.** Framework owns the input path per Phase 12 (`dismiss_chain`, `on_navigation`, `try_consume_cycle_step`); Phase 15 wires the inbound key through these hooks. When current focus is `FocusedPane::Framework(FrameworkFocusId::Toasts)`, dispatch runs the focused-Toasts chain in this order:
  1. **Pre-globals**: if the inbound key matches the live keymap entry for `GlobalAction::NextPane`, call `framework.toasts.try_consume_cycle_step(CycleDirection::Next)`; for `PrevPane`, call it with `CycleDirection::Prev`. If it returns `true` (scroll room), consume the key and stop. Otherwise continue. The `CycleDirection` arg is determined by which matched action's keymap entry the inbound key hit; the hook never runs for any other key.
  2. **Framework globals** (incl. `Dismiss`) — `GlobalAction::Dismiss` flows through `dismiss_chain → dismiss_framework → toasts.dismiss_focused()`. `dismiss_chain` calls `reconcile_focus_after_toast_change(ctx)` automatically when the toast vec drops to empty (Phase 12), so dispatch-time post-dismiss focus repair needs no extra Phase 15 call site.
  3. **App globals**.
  4. **Focused-pane scope** — `ToastsAction` is empty in Phase 12 (Phase 21 adds `Activate`); this slot is reserved in the chain order so Phase 21 can wire `Activate` without restructuring dispatch.
  5. **Resolved navigation**: if the inbound key resolves to a `NavigationAction` through the keymap, translate to `ListNavigation` via `Navigation::list_navigation` (default impl matches against the trait's `UP`/`DOWN`/`HOME`/`END` constants) and call `framework.toasts.on_navigation(list_nav)`. Pure pane-local viewport mutation; no `&mut Ctx` needed.
  6. **Unhandled** — drop.

  Phase 14 has no Toasts work — the framework already owns the input path. `ToastsAction::Dismiss` is gone (Phase 12); Esc-on-Toasts flows through `GlobalAction::Dismiss`. The borrow trap that Phase 11's review flagged is avoided structurally: every focused-Toasts entry point takes `&mut self` only.

**`KeyOutcome` enum (introduced in Phase 9, broadened in Phase 15).** Public, two-variant: `Consumed` (pane handled the key; caller stops dispatch), `Unhandled` (caller continues to the globals chain / dismiss fallback). First defined in Phase 9 as the return type of `RuntimeScope::dispatch_key` (app-pane dispatch path, surfaced publicly through `Keymap::dispatch_app_pane`). Phase 11 re-uses the same enum on framework-pane inherent `handle_key` methods so the dispatch loop reads one return type across both surfaces. Boolean would compile, but standing rule "enums over `bool` for owned booleans" applies — the return is a domain decision (handled vs not handled), not a generic flag.

**Phase 15 tests:**
- Rebinding `FinderAction::Cancel` to `'q'` closes finder; `'k'` typed in finder inserts `'k'` even with vim mode on.
- Binding any action to `Up` while in Awaiting capture mode produces a "reserved for navigation" rejection (replaces today's `is_navigation_reserved` semantics via scope lookup).

**Framework-internal action enums are not subject to the 14.3.5 form choice.** `ToastsAction`, `SettingsAction`, `KeymapPaneAction`, and the hand-rolled `tui_pane::GlobalAction` were all defined before the macro grew the 2-positional shorthand. Leave them as they are unless a chunk adds a new variant (in which case follow the same "2-positional unless bar_label differs" rule for the new variant only). Phase 15 adds no variants — it only wires handlers — so no form change applies.

### Retrospective

**What worked:**
- Framework `SettingsPane::handle_key` and `KeymapPane::handle_key` now resolve `bind` against `Self::defaults().into_scope_map().action_for(...)` and flip `EditState`: `Browse → Editing` / `Browse → Awaiting` on `StartEdit`, back to `Browse` on `Save`/`Cancel`. The `#[allow(dead_code)]` on `EditState::Editing` (settings) and `EditState::Awaiting` (keymap) lifted because both are now constructed in `handle_key`.
- New transition tests added in both panes (settings: `enter_in_browse_transitions_to_editing`, `esc_in_editing_returns_to_browse`, `save_in_editing_returns_to_browse`; keymap: `enter_in_browse_transitions_to_awaiting`, `esc_in_awaiting_returns_to_browse`, `save_in_conflict_returns_to_browse`). 625 nextest tests + doctests pass; clippy `-D warnings` clean across the workspace; `cargo install --path .` replaced the binary cleanly.
- `keymap_capture_keys` and `settings_edit_keys` marker stubs stay `const fn` (clippy `missing_const_for_fn` gates the form per `feedback_const_opportunistic.md`); later phases make them suppression markers only while the framework panes expose command-returning mutation APIs.

**What deviated from the plan:** Phase 15 ships partial. Three pieces in the original plan are deferred to Phase 19 because each is structurally blocked on Phase 19's atomic dispatcher cutover:
- **`finder_keys` swap to `keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, ctx)`.** With the framework `FinderPane::dispatcher()` still a no-op (the standing Phase 14 contract through Phase 18), `dispatch_app_pane` returns `KeyOutcome::Consumed` for any registered Finder action key (Enter/Esc) but performs no state mutation. The "Consumed → halt; Unhandled → fall through to char/backspace/delete" pattern would silently break Finder Confirm/Cancel. The swap lands together with `FinderPane::dispatcher` getting a real body inside Phase 19's "Wire dispatchers" inventory. Today's `finder_keys` body (`super::finder::handle_finder_key(app, bind.code)`) keeps Finder working until then.
- **Toasts focus gate dispatch chain in `src/tui/input.rs`.** The plan's six-step chain (pre-globals `try_consume_cycle_step` → framework globals incl. `Dismiss` → app globals → `ToastsAction` slot → resolved navigation via `Navigation::list_navigation` → unhandled) requires `app.framework_keymap` to be the live source of truth for global/nav resolution — which Phase 18's "Load TOML into the framework keymap" + Phase 19's dispatcher cutover establishes. Until then the legacy `PaneBehavior::Toasts → handle_toast_key` arm at `src/tui/input.rs:148` keeps focused-toasts navigation/Enter alive, and `app.dismiss(target)` keeps Esc-dismiss alive through `handle_global_key`'s `GlobalAction::Dismiss` arm.
- **Per-setting buffer mutation / per-binding capture.** Phase 15 only flipped pane state. The final ownership split landed later: Phase 25 moved SettingsPane editing into framework-owned `handle_text_input`, and Phase 28 moved KeymapPane capture into `handle_capture_key(...) -> KeymapCaptureCommand`. Finder remains app-owned and keeps its free-function text-input handler.

**Surprises:**
- The framework's `SettingsPane`/`KeymapPane` are held inline on `Framework<Ctx>` (`framework.settings_pane`, `framework.keymap_pane`) and reached via `Framework::overlay()` — they are **not** registered in the `Keymap` builder. Their `defaults()` bindings live as in-memory `Bindings` returned per-call from the pane; runtime TOML rebinds against these overlays will need a separate registration story (parallel to `register::<Pane>` for app panes). Today the binary's `handle_settings_key` / `handle_keymap_key` legacy paths still own dispatch routing entirely.
- The plan's mandatory test list — "Rebinding `FinderAction::Cancel` to `'q'` closes finder" and "Binding any action to `Up` in Awaiting capture mode produces a reserved-for-navigation rejection" — both presuppose the deferred work above (TOML loading on the framework keymap + dispatcher wiring + overlay routing). Both move to Phase 23's regression suite after the Phase 21 overlay cleanup.

**Implications for remaining phases:**
- Phase 16's "Reroute base-pane navigation" stays unchanged in scope.
- Phase 18's "Load TOML into the framework keymap" bullet (line 3011) is unchanged but its acceptance test is now a hard prerequisite for the Phase 15 deferred items (the `OutputAction::Cancel` rebind test depends on `app.framework_keymap` being TOML-loaded; the `FinderAction::Cancel`/`Awaiting`-mode tests do too).
- Phase 19's "Wire dispatchers" inventory acquires three Phase-15-deferred items: (a) `finder_keys` swap to `dispatch_app_pane`; (b) Toasts focus gate chain in `src/tui/input.rs` replacing the `PaneBehavior::Toasts` arm + `handle_toast_key`; (c) overlay text-input suppression/routing for Settings/Keymap. Later phases replace the temporary bridge with command-returning framework pane methods. Each lands together with its corresponding dispatcher body and legacy-handler deletion in the Phase 19 atomic cutover.
- The current `SettingsPane::handle_key` / `KeymapPane::handle_key` re-resolve `bind` against `Self::defaults().into_scope_map()` on every keystroke. This is correct but transient: Phase 17's overlay scope registration gives `handle_key` a registered scope on `Keymap` rather than rebuilding `defaults()` per call. Future readers should not preserve the per-keystroke rebuild — no test or invariant depends on it; it is temporary scaffolding before the Phase 19 overlay dispatch cutover.

### Phase 15 Review

- **Phase 16 third test** re-scoped to "Phase 15 overlay handlers" funnel observation; the full no-parallel-write funnel test moves to Phase 23 after the Phase 22 focus cleanup.
- **Phase 18 structural Esc preflight** gained a "prerequisite verification" note pointing at `Keymap::key_for_toml_key` for primary reverse lookup; Phase 18 later added the all-bind-aware predicate required by the final structural check.
- **Phase 18 acceptance tests** rewritten as two pipeline tests (non-Output focus + Output focus, both routed through `src/tui/input.rs`'s normal key path against the structural preflight) plus a scaffolding warning that bans direct `dispatch_app_pane` use until Phase 19.
- **Phase 19 "Wire dispatchers" inventory** gained three new bullets (Phase-15-deferred (a) `finder_keys` swap with explicit fallback for `Char` / `Backspace` / `Up` / `Down` / `Home` / `End`; (b) Toasts focus gate chain replacing the `PaneBehavior::Toasts → handle_toast_key` arm; (c) overlay text-input routing); `Delete:` list gained two new targets (the three overlay short-circuits at `input.rs:126-137` and the `PaneBehavior::Toasts` arm at `:148`); the stale "do not delete `handle_finder_key`" note rewritten as "delete or extract helpers, depending on whether `FinderPane::dispatcher` + the new fallback reuse helpers."
- **Phase 17** added — bundled `tui_pane` cutover prerequisites: shared `[global]` coordination, overlay scope registration, `key_for_toml_key`, temporary overlay text-input routing, and paired-slot rendering.
- **Phase 17** expanded — added overlay scope registration (`register_settings_overlay` / `register_keymap_overlay`), public infallible accessors (`Keymap::settings_overlay` / `Keymap::keymap_overlay`), consumer-update bullets (bar render + Phase 19 dispatch read through the accessors), borrow-trap caution (resolution lives in the dispatch loop, not on the panes through `Ctx`), and four new acceptance tests covering existing TOML, rebinds, unknown tables, and unknown actions.

### Phase 16 — Reroute base-pane navigation (✅ landed)

`KeyCode::Up`/`Down`/`Left`/`Right`/`PageUp`/`PageDown`/`Home`/`End` in `handle_normal_key` (`input.rs:580-622`), `handle_detail_key`, `handle_lints_key`, `handle_ci_runs_key` consult `NavigationAction` after the pane scope. ProjectList's `Left`/`Right` route via `ProjectListAction::CollapseRow` / `ExpandRow` (pane-scope precedence). **Seam from 14.4c:** `src/tui/input.rs::handle_normal_key` already carries an `ProjectListAction::ExpandRow | ProjectListAction::CollapseRow => {}` no-op match arm (added so the migrated `ProjectListAction` compiled). Phase 16 deletes the top-of-`handle_normal_key` `KeyCode::Right` / `KeyCode::Left` arms (the ones that hardcode `app.expand()` / `app.project_list.collapse(...)`) and moves their bodies into that match arm — the arm exists, just needs its body filled. Delete `NAVIGATION_RESERVED` (`keymap.rs:794-799`) and `is_navigation_reserved` — replaced by scope lookup against `NavigationAction`.

**`App::set_focus` override lands here.** Once the framework keymap dispatches navigation, `framework.set_focused(...)` becomes the new write path; without an `App::set_focus` override the legacy `app.focus: Focus` field stops receiving updates and every pane-highlight / bar / render path that still reads it desyncs. Phase 16 lands an explicit `set_focus` body on `impl AppContext for App` that mirrors into the legacy field:

```rust
fn set_focus(&mut self, focus: FocusedPane<AppPaneId>) {
    self.framework.set_focused(focus);
    if let FocusedPane::App(id) = focus {
        self.focus.set(id.to_legacy());
    }
}
```

`AppPaneId::to_legacy()` already exists from 14.2. Framework focuses (`FocusedPane::Framework(FrameworkFocusId::Toasts)`) skip the legacy mirror — the legacy `app.focus` is `PaneId`-typed and `PaneId::Toasts` is owned by the framework path now. The override survives Phase 19. Phase 22 deletes the `self.focus.set(...)` mirror line, and may delete the override entirely if the trait default is enough after the legacy `app.focus` field is gone. Phase 23's funnel-test regression depends on all framework focus changes flowing through `AppContext::set_focus`.

**Phase 16 tests:**
- Rebinding `NavigationAction::Down` to `'j'` (vim-off) moves cursor.
- Rebinding `ProjectListAction::ExpandRow` to `Tab` (with `GlobalAction::NextPane` rebound away) expands current row.
- A test impl wrapping `set_focus` (counting calls) observes every focus change driven by the framework-routed paths that already exist (Phase 15 overlay handlers); the full funnel-test (no parallel write to `app.focus` anywhere in the binary) lands in Phase 23 after Phase 22 removes the direct focus writers.

### Retrospective

**What worked:**
- The pane-scope-first / navigation-scope-second precedence collapsed cleanly into all four legacy handlers (`handle_normal_key`, `handle_detail_key`, `handle_lints_key`, `handle_ci_runs_key`) without re-introducing the deleted `KeyCode::Up | Down | Home | End` hardcoded arms.
- Deleting `NAVIGATION_RESERVED` / `is_navigation_reserved` / `KeymapErrorReason::ReservedForNavigation` together with the two reservation tests was the right scope — the legacy loader stopped rejecting bare arrows, and the new precedence (pane scope wins, framework navigation second) makes the reservation check structurally unnecessary.
- The `App::set_focus` override compiles and dispatches both the framework write (`self.framework.set_focused(focus)`) and the legacy mirror (`self.focus.set(id.to_legacy())` for `FocusedPane::App`); framework-only focuses skip the mirror as planned.

**What deviated from the plan:**
- The Phase 16 framework-keymap rebinding test (`NavigationAction::Down` → `'j'`) needed `tui_pane`-side load_toml infrastructure to inject. Phase 18 owned the production `App::new`-side `load_toml` wiring; Phase 16 temporarily added a `#[cfg(test)] pub(super) fn build_framework_keymap_with_toml(...)` sibling helper inside `src/tui/framework_keymap.rs` so the test could rebuild the keymap with a temp TOML and overwrite `app.framework_keymap`. Phase 18 retired this helper once the production builder consumed `keymap_path()` directly.
- The legacy `ProjectListAction::ExpandRow` defaults stayed at `Shift+Right` / `Shift+Left` (the comment was rewritten to say "avoid colliding with the navigation defaults" instead of "pass `is_navigation_reserved`"). Switching them to bare arrows would fight the new framework `NavigationAction::Right` / `Left` defaults — keeping Shift modifiers preserves both routes.
- `handle_detail_key` rewrote the pane-scope match into an `is_some_and(|action| { match … ; true })` form so the consumed flag drives the post-match navigation fallthrough cleanly. This is a structural rewrite of the match, not just a body insertion — the original Up/Down/Home/End block at the top of the function has been deleted along with the inner `match app.focus.base()` early-returns.

**Surprises:**
- `crate::keymap::KeyBind` and `tui_pane::KeyBind` are distinct types (the former normalises uppercase Char + SHIFT → uppercase, the latter is plain). The Phase 16 navigation lookup constructs a `tui_pane::KeyBind` from the local `bind`'s code/modifiers (`framework_bind`) so `Keymap::navigation::<AppNavigation>().action_for(...)` resolves correctly. Phase 19's atomic cutover should consolidate on one `KeyBind` type at the dispatch boundary.
- `cargo mend` hoisted `use std::path::PathBuf` to the top-level after the inline `use` was added inside `build_framework_keymap_with_toml`. The helper is `#[cfg(test)]`, so the top-level import errored on non-test builds; the fix is `#[cfg(test)] use std::path::PathBuf;`.

**Implications for remaining phases:**
- **Phase 18 retired the test-only helper.** `build_framework_keymap_with_toml` existed only because Phase 16's rebinding test needed `load_toml` before Phase 18 wired it into `App::new`. The test now uses the production loader path.
- **Phase 23's funnel test depends on the Phase 22 focus cleanup.** The override is now the single mutation funnel for framework-routed focus writes; Phase 23's "no parallel write to `app.focus` anywhere in the binary" assertion requires that every other `app.focus.set(...)` call has either been deleted or re-routed through `app.set_focus(FocusedPane::App(...))` in Phase 22. Phase 22 should grep for residual `app.focus.set(` call sites at start.
- **Phase 19 dispatcher cutover semantics for `Mode::TextInput`.** Phase 16's Phase 15 overlay handlers (`SettingsPane::handle_key` / `KeymapPane::handle_key`) still flip `EditState` only. Phase 19 established text-input suppression/routing at cutover; Phase 25 and Phase 28 then moved Settings/Keymap mutation state into framework-owned command APIs.
- **Phase 18 replaced the legacy `Esc`-on-output preflight.** Phase 16 left that pre-handler intact; Phase 18 rewrote it to use the framework keymap's all-bind-aware Output-cancel predicate. The Phase-15-deferred `finder_keys` swap and Toasts focus gate chain are still Phase 19 deliverables; nothing in Phase 16 unblocks them earlier.

### Phase 17 — `tui_pane` cutover prerequisites ✅

Phase 17 is one additive `tui_pane` bump that lands before the binary starts consuming framework-keymap TOML in Phase 18. It bundles the library prerequisites that otherwise block the cutover in different places: shared `[global]` TOML coordination, framework overlay scope registration, reverse lookup by `(AppPaneId, toml_key)`, temporary text-input routing for framework overlays, and paired-slot rendering. No binary legacy dispatch path is deleted here.

**Bundle rule.** Default plan: land all five `tui_pane` changes in one bump, because Phase 18, Phase 21, and Phase 23 all need them and each change is additive. At implementation start, the implementer may split text-input routing or paired-slot rendering into separate commits for review clarity, but the phase remains one dependency gate: Phase 18 does not begin until all five are available.

**Shared `[global]` table coordination.** The framework-keymap loader currently hard-rejects unknown action keys in the registered enum. That breaks cargo-port's existing TOML schema, where one `[global]` table mixes framework-owned keys (`quit`, `restart`, `next_pane`, `prev_pane`, `open_keymap`, `open_settings`, `dismiss`) with app-owned keys (`find`, `open_editor`, `open_terminal`, `rescan`). Decision: preserve the existing schema with per-key skip-when-peer-registered behavior. Each globals overlay accepts its own keys and ignores keys known to the peer globals enum; keys known to neither enum still raise `KeymapError::UnknownAction`.

Implementation sketch: track the app-globals enum's recognized `toml_key()` set at `register_globals` time. During both `apply_toml_overlay::<tui_pane::GlobalAction>("global", ...)` and the app-globals overlay, an otherwise unknown key is skipped only if the peer enum recognizes it. With no app globals registered, behavior remains strict: every unknown `[global]` key errors.

**Overlay scope registration for `[settings]` and `[keymap]`.** `SettingsPane` and `KeymapPane` are framework-owned overlay panes, not app-pane registrations, so `load_toml(...) + register::<P>(...)` has no consumer for user `[settings]` / `[keymap]` tables. Add explicit overlay registrations:

```rust
impl<Ctx: AppContext> KeymapBuilder<Ctx, /* configuring state */> {
    pub fn register_settings_overlay(self) -> Result<Self, KeymapError>;
    pub fn register_keymap_overlay(self) -> Result<Self, KeymapError>;
}

impl<Ctx: AppContext> Keymap<Ctx> {
    pub fn settings_overlay(&self) -> &ScopeMap<SettingsPaneAction>;
    pub fn keymap_overlay(&self) -> &ScopeMap<KeymapPaneAction>;
}
```

Each registration calls the pane defaults once, stores the resolved `ScopeMap` under the legacy table name (`"settings"` / `"keymap"`), marks the table name as known for TOML validation, and applies any TOML overlay. The final `Keymap<Ctx>` always exposes default overlay scopes; the registration methods are what let user TOML override them. Do not thread the keymap into the overlay panes through `Ctx`; the dispatch loop already owns the `&Keymap<Ctx>` read and can pass the resolved action into the pane handler.

**Reverse-lookup public method.** Confirm or add `pub fn key_for_toml_key(&self, app_pane_id: Ctx::AppPaneId, action_toml_key: &str) -> Option<KeyBind>` on `Keymap<Ctx>`. It walks the app-pane registry by `AppPaneId` and reverse-walks the resolved `ScopeMap` for the action whose `toml_key()` matches. Phase 18's structural Output-cancel preflight depends on this method. This stays within the post-Phase-9-reset rule because it is keyed by `(AppPaneId, &str)`, not by a public typed `<P>` probe.

**Temporary text-input routing.** Phase 17 introduced a short-lived binary hook for framework overlay text input so the Phase 19 cutover could proceed before Settings/Keymap state fully moved into `tui_pane`. That bridge is not the final API: Phase 25 moved SettingsPane edit-buffer mutation into the framework, and Phase 28 moves KeymapPane capture state into the framework through `KeymapPane::handle_capture_key(...) -> KeymapCaptureCommand`. The current source of truth is command-returning framework pane methods, not binary-side handler storage.

**Paired-slot rendering.** Preserve secondary actions from `BarSlot::Paired(a, b, label)` all the way to rendered bar output. Today `PaneScope::render_bar_slots` collapses every paired slot to the primary action via `.primary()`, so Phase 23 cannot assert `+/-`, `left/right expand`, or vim-mode paired labels. Land one concrete representation in this phase: either extend `RenderedSlot` with an optional secondary key, add a parallel pre-flattened span render method, or introduce a `RenderedSlot::Paired { primary, secondary, label }` variant. Whichever representation is chosen, `bar/nav_region.rs` and `bar/pane_action_region.rs` must route paired slots through `bar/support.rs::push_paired` instead of `push_slot`.

**Phase 17 tests:**
- Combined `[global]` TOML loads when both framework globals and an app globals enum are registered; framework-owned and app-owned rebinds both resolve; a truly unknown `[global]` key still errors.
- `[settings]` and `[keymap]` TOML tables load when their overlay registrations are called; representative rebinds resolve through `settings_overlay()` / `keymap_overlay()`; unknown overlay tables and unknown actions under known overlay tables still error.
- `key_for_toml_key` round-trips against a resolved app-pane scope.
- Text-input bridge coverage was temporary. After Phase 25/28, SettingsPane and KeymapPane tests assert the framework-owned command APIs (`handle_text_input`, `handle_capture_key`) instead of handler replacement.
- Paired bar-slot rendering preserves primary key, secondary key, and shared label text through the runtime-scope render path.

**Risk section.** With shared `[global]` coordination and overlay scope registration in Phase 17, the `## Risks and unknowns` claim that existing user TOML configs have no breaking change stays valid through Phase 19. Without this phase, Phase 18 cannot safely load TOML and Phase 19 would silently drop `[settings]` / `[keymap]` rebinds.

### Retrospective

**What worked:**
- The five library prerequisites landed as additive `tui_pane` API at the time: shared `[global]` peer-key skipping, `register_settings_overlay` / `register_keymap_overlay`, overlay scope accessors, a temporary text-input bridge, and paired-slot rendering. Later phases removed the bridge after the framework panes owned their own mutation state.
- The builder tests caught the important compatibility cases: mixed `[global]` tables load when both global enums are registered, truly unknown global keys still error, and `[settings]` / `[keymap]` rebinds only apply when their overlay scopes are registered.
- The flat `RenderedSlot` shape survived the paired-slot work by adding `secondary_key: Option<KeyBind>`; `bar/support.rs::push_slot` now routes paired slots through `push_paired` without changing region-module ownership.

**What deviated from the plan:**
- `Keymap::key_for_toml_key` already existed from earlier phases, so Phase 17 confirmed and covered it rather than adding a new method.
- The paired-slot representation chosen was `RenderedSlot { secondary_key: Option<KeyBind>, label }`, not a full enum or "secondary triple." The third `BarSlot::Paired` field is the shared label rendered after `primary/secondary`, not a separate separator.
- Overlay bar rendering now reads `Keymap::settings_overlay()` / `keymap_overlay()` instead of rebuilding `SettingsPane::defaults()` / `KeymapPane::defaults()` per frame. The same resolved-scope source should be used by Phase 19's overlay dispatch path.

**Surprises:**
- Clippy's `struct_field_names` lint rejected a private `Keymap` field named `keymap_overlay` / `keymap_pane_overlay`; the storage field is `overlay_keymap_scope` while the public accessor remains `keymap_overlay()`.
- The plan text around framework overlay text-input handlers still carried old phase-number language. Code comments were tightened to state that Phase 17 provides injection, while Phase 19 wires cargo-port's mutation bodies.

**Implications for remaining phases:**
- Phase 18 can now load the user's TOML through the framework keymap without breaking existing mixed `[global]`, `[settings]`, or `[keymap]` tables.
- Phase 19 should route framework Settings / Keymap overlay dispatch through the resolved overlay scopes on `Keymap`, not through fresh `defaults().into_scope_map()` calls.
- Phase 23's paired rebind snapshots can assert full `left/right` and `+/-` rendered rows against `render_status_bar`; no extra paired-slot prerequisite remains.

### Phase 17 Review

- **Phase 18 structural Output-cancel preflight** now requires an all-bind-aware reverse lookup (`is_key_bound_to_toml_key` or equivalent) instead of primary-only `key_for_toml_key`, with an acceptance test for `[output] cancel = ["Esc", "q"]`.
- **Phase 18 `KeyBind` bridge** now explicitly constructs a temporary `tui_pane::KeyBind` from the legacy input bind before querying `app.framework_keymap`; Phase 19 deletes that bridge during KeyBind consolidation.
- **Phase 19 overlay dispatch** now resolves Settings / Keymap overlay actions through `app.framework_keymap.settings_overlay()` / `keymap_overlay()` before mutating panes, so TOML rebinds drive both bar rendering and dispatch.
- **Phase 23 rebind tests** now use the production TOML loader only; the stale pre-Phase-18 helper paragraph was removed.
- **Phase 24 Toasts bar renderer** now includes `secondary_key: None` in the `RenderedSlot` construction plan.
- **Phase 25 `[toasts]` TOML surface** now states that shared-`[global]` peer skipping does not apply to the toast settings table; unknown-key handling stays local to `[toasts]`.

### Phase 18 — Reroute Output, structural Esc (✅ landed)

Phase 18 is the first binary phase that consumes the framework keymap for user-facing behavior. It wires production TOML loading into `app.framework_keymap`, replaces the hardcoded Output-cancel structural preflight with a framework reverse lookup, and retires the Phase 16 test-only TOML helper. Broad dispatcher cutover and legacy handler deletion remain Phase 19.

Phase 12 added the framework-owned typed `Toasts<Ctx>` but did not delete the binary's `handle_toast_key` (`input.rs:657-684`); cargo-port's `app.toasts: ToastManager` still drives that handler until Phase 26 migrates the manager into `tui_pane`. Focused-toasts dismiss already flows through `GlobalAction::Dismiss -> dismiss_chain -> dismiss_framework -> toasts.dismiss_focused()` (Phase 12), but the binary's parallel dismiss path through `app.toasts` stays in place until the manager migration deletes both.

**Prerequisites.** Phase 17 must have shipped `key_for_toml_key`, shared `[global]` coordination, and overlay-scope registration. If any of those APIs are missing, finish Phase 17 before drafting this binary change.

**Structural reverse lookup must be all-bind aware.** Before replacing the Output-cancel preflight, add `Keymap::is_key_bound_to_toml_key(id, action_toml_key, &bind) -> bool` (or an equivalent `keys_for_toml_key`) and use that predicate instead of `key_for_toml_key(...) == Some(bind)`. `key_for_toml_key` returns the primary binding only, which is not enough for structural checks. Acceptance test: `[output] cancel = ["Esc", "q"]` clears `example_output` on both Esc and `q`.

**Load TOML into the framework keymap.** Update `src/tui/app/construct.rs::AppBuilder<Started>::build` so the framework-keymap builder consumes the same `keymap::keymap_path()` TOML as the legacy `ResolvedKeymap` path before any framework-keymap behavior is user-visible. The chain order is:

1. `load_toml(keymap_path).with_context(|| format!("loading keymap from {}", keymap_path.display()))?`
2. `register_navigation::<AppNavigation>()`
3. `register_globals::<AppGlobalAction>()`
4. `register_settings_overlay()` and `register_keymap_overlay()`
5. all app-pane registrations
6. `build_into(&mut app.framework)`

The `.with_context(...)` wrapper is required so `App::new` errors include the TOML path. Phase 17's shared-`[global]` handling is what prevents existing mixed `[global]` tables from erroring during this load.

**Structural Output-cancel preflight.** The current Esc-on-output pre-handler at `src/tui/input.rs:112-119` runs before overlays, globals, pane handlers, and Toasts focus handling. Preserve that cross-pane behavior, but compare the inbound bind against the framework keymap's resolved Output cancel bind:

```rust
let bind = KeyBind::from(event);
let framework_bind = tui_pane::KeyBind {
    code: bind.code,
    mods: bind.modifiers,
};
if !app.inflight.example_output().is_empty()
   && !matches!(app.framework.focused_pane_mode(app), Some(Mode::TextInput(_)))
{
    if app.framework_keymap.is_key_bound_to_toml_key(
        OutputPane::APP_PANE_ID,
        OutputAction::Cancel.toml_key(),
        &framework_bind,
    ) {
        let was_on_output = app.focus.is(PaneId::Output);
        app.inflight.example_output_mut().clear();
        if was_on_output { app.focus.set(PaneId::Targets); }
        return;
    }
}
```

`OutputAction::Cancel.toml_key()` resolves through `tui_pane::Action`; add `use tui_pane::Action;` in `src/tui/input.rs` if needed. The `Mode::TextInput` guard keeps Esc available for Settings edit cancel, Keymap capture cancel, and Finder text input. Focused Toasts is `Mode::Navigable`, so this preflight still wins over Toast dismiss when `example_output` is non-empty, matching today's ordering.

The `framework_bind` conversion is a temporary Phase 18 bridge across the legacy `crate::keymap::KeyBind` / framework `tui_pane::KeyBind` split. Phase 19 deletes this bridge when it standardizes post-cutover dispatch on `tui_pane::KeyBind`; do not introduce new normalization behavior here.

**Acceptance tests:**
- With temp TOML `[output]\ncancel = "q"\n`, build `App` through the normal `App::new` path, focus a non-Output pane, push `example_output`, send `'q'` through `src/tui/input.rs`, and assert output is cleared while focus is unchanged.
- Same TOML, but focus Output before sending `'q'`; assert output is cleared and focus moves to Targets.
- With Settings in Editing mode and `example_output` non-empty, pressing Esc cancels the edit instead of clearing `example_output`.
- Retargeted the Phase 16 navigation rebind test to the production loader, then deleted `#[cfg(test)] pub(super) fn build_framework_keymap_with_toml(...)` from `src/tui/framework_keymap.rs`.

**Test scaffolding warning.** Phase 18 tests must not call `app.framework_keymap.dispatch_app_pane(OutputPane::APP_PANE_ID, ...)` directly. `OutputPane::dispatcher()` remains no-op until Phase 19, so direct dispatch bypasses the structural preflight and tests the wrong path. The Output-focused dispatcher-path test lands in Phase 19 after `OutputPane::dispatcher()` is wired.

### Retrospective

**What worked:**
- `Keymap::is_key_bound_to_toml_key(id, toml_key, bind)` landed as the all-bind-aware reverse lookup Phase 18 needed. `key_for_toml_key` stays the primary-key API for display-oriented callers.
- Production App construction now builds the framework keymap from the same `keymap::keymap_path()` TOML as the legacy `ResolvedKeymap` path, with `.load_toml(...)` running before navigation, globals, overlay scopes, and app-pane registrations.
- The Phase 16 test-only `build_framework_keymap_with_toml(...)` helper was deleted. Rebind tests now use a test-only `keymap::override_keymap_path_for_test(...)` guard and then build `App` through the normal `App::new` path.
- The structural Output-cancel preflight now resolves `OutputAction::Cancel.toml_key()` through `app.framework_keymap` and honors multi-bind TOML such as `cancel = ["Esc", "q"]`.
- Existing cargo-port TOML uses `[global] settings = "s"`, while the framework canonical action is `open_settings`. `tui_pane::GlobalAction::from_toml_key` now accepts `settings` as a legacy alias for `OpenSettings`, and the shared `[global]` peer-key skip set includes the alias so app-globals loading does not reject existing configs.

**What deviated from the plan:**
- `src/tui/framework_keymap.rs::build_framework_keymap` now accepts a configured `KeymapBuilder<App, Configuring>` instead of constructing the builder internally. That lets `construct.rs` wrap `load_toml(path)` with the required path-bearing `anyhow::Context` before the shared registration chain runs.
- The text-input guard checks both `app.framework.focused_pane_mode(app)` and the legacy overlay flags (`finder`, `settings editing`, `keymap awaiting`). Phase 18 still runs before overlay dispatch is fully mirrored into framework overlay focus, so this bridge preserves today's Esc/text-entry behavior until Phase 19 consolidates the entry flow.

**Implications for remaining phases:**
- Phase 19 can assume `app.framework_keymap` is production-loaded and overlay scopes are registered; it should delete the temporary legacy-to-framework `KeyBind` bridge when dispatch standardizes on `tui_pane::KeyBind`.
- Phase 23 rebind snapshots should use the production loader path only. No test should recreate the deleted `build_framework_keymap_with_toml(...)` bypass.

### Phase 18 Review

- **Phase 19 `VimMode` wiring:** the framework keymap must map cargo-port's `NavigationKeys` config into `tui_pane::VimMode` before cutover, including any framework-keymap rebuild / reload path. Phase 23 gets production-path tests for `ArrowsAndVim` vs `ArrowsOnly`.
- **Framework globals:** Phase 19 routes framework-global hits through the shipped `app.framework_keymap.dispatch_framework_global(action, app)` API. It does not add a second `tui_pane::GlobalAction::dispatcher` concept.
- **Command-key invariant wording:** command shortcuts dispatch through the keymap, but structural preflights, modal confirmation, and text-input editing fallback may still inspect literal keys. Those are explicit survivors, not cleanup failures.
- **Output cancel ordering:** Phase 19 preserves Phase 18's structural Output-cancel preflight as an `is_key_bound_to_toml_key(...)` lookup. It does not fold cross-pane `example_output` clearing into `OutputPane::dispatcher()`.
- **Minor cleanups:** Phase 23 rebind tests now speak in production-loader terms, Phase 19 names the text-input bridge removal criteria, and malformed TOML binding tests assert `KeymapError::InvalidBinding { source, .. }` for the production loader path.

### Phase 19 — Bar swap and cleanup

**Framework-side cleanup landed in Phase 12.** `FrameworkPaneId` (split into `FrameworkOverlayId` + `FrameworkFocusId`), `Framework::dismiss` (renamed `dismiss_framework`), `Toasts::try_pop_top`, `ToastsAction::Dismiss`, the `Vec<String>` placeholder, and `Mode::Static` for Toasts are all already gone. Phase 19 deletes binary-side artifacts only.

Add the `What dissolves` / `What survives` summary (currently in this doc) as user-facing notes inside `tui_pane/README.md` so the published library has its own change log of what the framework absorbed.

**Binary main loop change (post-Phase-3 review).** The binary's main loop in `src/tui/terminal.rs` switches from polling `app.overlays.should_quit()` to polling `app.framework.quit_requested()` and `app.framework.restart_requested()`. The `should_quit()` accessor on `overlays` deletes; the framework owns the lifecycle flags now. If the binary needs cleanup, it registers `.on_quit(|app| { app.persist_state() })` on the builder. **Land together.** The main-loop poll-site swap and the `tui_pane::GlobalAction::Quit` / `Restart` dispatcher bodies (per the "Wire dispatchers" sub-section above) must land in the same Phase 19 cutover changeset. The dispatchers write `app.framework.request_quit()` / `request_restart()`; the loop reads `app.framework.quit_requested()` / `restart_requested()`. Splitting them leaves the binary unable to quit or restart. The post-14.8 `match App::new(...) { Err(e) => { tracing::error!(...); ExitCode::FAILURE }, Ok(app) => app }` block at `src/tui/terminal.rs:165-180` is unaffected by Phase 19 — only the loop-body poll moves; the construction-error envelope stays.

**Re-route any existing pre-quit / pre-restart cleanup paths into the keymap-builder hooks.** Phase 10 shipped `KeymapBuilder::on_quit(fn(&mut Ctx))` / `on_restart(fn(&mut Ctx))` / `dismiss_fallback(fn(&mut Ctx) -> bool)` — Phase 19 walks the binary for any code that runs on quit/restart (state persistence, watcher shutdown, terminal-cleanup hooks beyond what ratatui handles) and moves those bodies into closures registered on the builder during keymap construction. The post-Phase-19 binary touches the lifecycle flags only by reading them from the main loop; mutation flows exclusively through the framework's `GlobalAction` dispatcher.

**Wire framework `VimMode` from cargo-port config before cutover.** Phase 18 production-loads the framework keymap but leaves the builder's `VimMode` at its default unless Phase 19 maps `config.tui.navigation_keys` into `tui_pane::VimMode`. Before framework dispatch becomes authoritative, the production builder chain must call `.vim_mode(...)` from cargo-port's `NavigationKeys` value (`ArrowsAndVim` → `VimMode::Enabled`, `ArrowsOnly` → `VimMode::Disabled`). Apply the same mapping anywhere Phase 19 rebuilds or reloads `app.framework_keymap`; startup-only wiring is not enough if config/keymap reload can replace the resolved scope after cutover.

**Wire dispatchers.** Every app-side `dispatcher()` fn that 14.2–14.7 left as `|_, _| {}` flips to a real body in this phase. Phase 19 is not only cleanup — it is where the framework path becomes live. Each dispatcher routes its action(s) to the existing or extracted operation body that today's `handle_*_key` arms invoke; the helper names below are illustrative anchors, not commitments to specific symbol names (the implementer extracts or reuses what's already there). Required inventory:
- **Framework-owned globals route through the existing `tui_pane` API.** Do not add a new `tui_pane::GlobalAction::dispatcher` concept. The shipped library already exposes `Keymap::dispatch_framework_global(action, ctx)`, backed by `framework::dispatch_global(...)`; Phase 19's input chain resolves framework-global hits and calls `app.framework_keymap.dispatch_framework_global(action, app)`. That existing API owns `Quit`, `Restart`, `OpenSettings`, `OpenKeymap`, `Dismiss`, `NextPane`, and `PrevPane`.
- **`AppGlobalAction::dispatcher`** — bodies for the four variants `Find`, `OpenEditor`, `OpenTerminal`, `Rescan`. Each routes to the existing operation body invoked by today's `handle_global_key` arms for these variants.
- **`AppNavigation::dispatcher`** — routes `(NavigationAction, FocusedPane)` tuples to the legacy arrow-key bodies that Phase 16 relocated (the ones moved into the `ProjectListAction::ExpandRow | CollapseRow` match arm and into each per-pane handler's navigation arms).
- **Every app pane dispatcher** — `PackagePane`, `GitPane`, `TargetsPane`, `LintsPane`, `CiRunsPane`, `LangPane`, `CpuPane`, `ProjectListPane`, `OutputPane`, `FinderPane`. Each routes its action variants to the bodies executed today by the corresponding legacy handler arms (`handle_normal_key` / `handle_detail_key` / `handle_lints_key` / `handle_ci_runs_key` / `handle_finder_key` / `handle_output_key`, etc., on `Activate` / `Clean` / variant-specific operations).
- **Phase-15-deferred (a): `finder_keys` body swap to scope dispatch.** `src/tui/framework_keymap.rs::finder_keys` swaps from today's `super::finder::handle_finder_key(app, bind.code)` delegate to `match keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, ctx) { KeyOutcome::Consumed => {}, KeyOutcome::Unhandled => /* fallback */ }`. **Fallback must preserve all existing finder text-input + result-navigation behavior** that is not represented by a `FinderAction` variant: `Char(c)` appends to the query buffer, `Backspace` deletes from it, **`Up` / `Down` / `Home` / `End` navigate the finder result list** (these are not `FinderAction` variants today and must keep working). Arrow-key handling on `Unhandled` matches today's `handle_finder_key` arms one-to-one. **Do not add `Delete`** unless that is an intentional new behavior — `handle_finder_key` does not handle `Delete` today. Lands together with `FinderPane::dispatcher()` getting a real body in this same changeset.
- **Phase-15-deferred (b): Toasts focus gate chain** added to `src/tui/input.rs::handle_key_event`. Implements the six-step chain from Phase 15 (pre-globals → framework globals → app globals → ToastsAction slot → resolved navigation → unhandled) when current focus is `FocusedPane::Framework(FrameworkFocusId::Toasts)`. Replaces the `PaneBehavior::Toasts → handle_toast_key` arm at `src/tui/input.rs:148`.
- **Phase-15-deferred (c): framework-pane text-input mutation path.** The Phase 19 cutover needed Settings/Keymap overlay text input to suppress global/nav dispatch while still mutating the active overlay state. The final ownership model no longer stores binary handlers on the framework panes: SettingsPane owns edit-buffer mutation through `handle_text_input`, and Phase 28 moves KeymapPane capture state into `handle_capture_key(...) -> KeymapCaptureCommand`. Finder remains app-owned and keeps its `Mode::TextInput(finder_keys)` path.
- **Resolved overlay dispatch, not pane-local defaults.** Framework Settings / Keymap overlay input must resolve `bind` against `app.framework_keymap.settings_overlay()` / `keymap_overlay()` first, then apply the resolved `SettingsPaneAction` / `KeymapPaneAction` to the pane. Do not call `SettingsPane::handle_key` / `KeymapPane::handle_key` as the final dispatch path unless those methods are first refactored to accept a resolved action or a `&ScopeMap`; their current implementation rebuilds `Self::defaults().into_scope_map()` and would ignore user TOML. Acceptance test: rebind `[settings] start_edit = "F2"` and `[keymap] cancel = "F3"`; the bar and dispatch path both honor the rebound keys.
- **`KeyBind` type consolidation at the dispatch boundary.** Phase 19 standardizes post-cutover dispatch on `tui_pane::KeyBind`. Audit every `crate::keymap::KeyBind` call site into one of three buckets: delete with `ResolvedKeymap`, migrate to `tui_pane::KeyBind`, or remain as an explicit survivor for a non-framework concern (for example keymap UI / settings code that has not moved yet). The four temporary `framework_bind = tui_pane::KeyBind { code: bind.code, mods: bind.modifiers }` shims at `src/tui/input.rs:651-654` and `src/tui/panes/actions.rs:109-112, 228-231, 325-328` delete once those handlers receive `tui_pane::KeyBind` directly. **Event-vs-stored canonicalization decision:** before deleting the shims, settle parity for `BackTab` / `Shift+Tab` and shifted letters such as restart's `R`, and add regression tests for the chosen behavior. Do not copy legacy `crate::keymap::KeyBind::new` normalization wholesale: `tui_pane` intentionally dropped the legacy `+` / `=` collapse. If shifted-letter normalization changes the framework `KeyBind` policy, update parser, defaults, display, and tests together as an explicit framework key policy change rather than hiding it inside the cutover.

**Atomic cutover constraint.** The dispatcher bodies and the deletion of the legacy dispatch reads (`handle_*_key` fn bodies + their call sites + the `app.keymap.current().<scope>` accessors) must land together in the same Phase 19 cutover changeset. Splitting the change leaves the binary in one of two failure modes: both paths fire on the same key (double-dispatch), or neither path fires (dead key). The "Wire dispatchers" inventory above and the `Delete:` list below describe halves of the same atomic swap — they cannot be sequenced apart.

**Phase 19 `handle_key_event` entry-flow inventory.** This is the canonical target flow for `src/tui/input.rs::handle_key_event` after the atomic cutover. The `Delete:` block below names the removed legacy bodies / call sites; this inventory names the surviving entry order those deletions produce. "All command dispatch routes through the keymap" does not forbid the structural preflights, modal confirmation, or text-input fallback listed here from inspecting concrete keys — those are explicit survivors, not cleanup failures.

1. Input normalization / `KeyBind` construction under the Phase 19 `KeyBind` policy above.
2. Structural preflights: Esc kills a running example; `OutputAction::Cancel` reverse-lookup clears `example_output` and returns focus to Targets when appropriate. The Output-cancel branch remains a structural lookup through `app.framework_keymap.is_key_bound_to_toml_key(...)`; do not replace it with `OutputPane::dispatcher()` during cutover.
3. `handle_confirm_key(app, code)` survives unchanged at this position. Modal confirmation is binary-specific behavior, not framework keymap dispatch.
4. Overlay / text-input interception: framework Keymap / Settings overlays handle and return; Finder's app-side text-input path routes through `finder_keys` and preserves the text-entry fallback.
5. Framework globals, then app globals. `GlobalAction::Dismiss` runs `dismiss_chain`; unhandled keys do not run `dismiss_chain`.
6. If focused on Toasts, run the focused-Toasts chain from the Phase-15-deferred note and stop/drop on unhandled.
7. If focused on an app pane, dispatch the focused pane scope first.
8. Navigation fallback second, via `AppNavigation::dispatcher`.
9. Unhandled drops.

**Remove the Phase 18 text-input bridge only after framework overlay focus owns mode.** Phase 18's Output-cancel guard still checks legacy overlay flags (`finder`, Settings editing, Keymap awaiting) alongside `app.framework.focused_pane_mode(app)` because those modes are not fully represented by framework overlay focus yet. Phase 19 deletes those guard arms only after Settings editing, Keymap awaiting, and Finder input route through framework overlay focus / `Mode::TextInput`. Add regression coverage that Esc in Settings editing and Keymap awaiting, plus Finder text input, do not clear `example_output`.

**Visibility note.** 14.8 narrowed `framework_keymap::build_framework_keymap` from `pub(crate)` to `pub(super)` (only `construct.rs` calls it). If Phase 19's "Wire dispatchers" extractions move any caller outside `tui::app` — e.g. into `src/tui/input.rs` or a new top-level module — re-widen the visibility to `pub(crate)` or `pub(super)` against the right module. Mechanical edit; no semantic change.

Delete:

- `App::enter_action`, `shortcuts::enter()` const fn.
- The old combined `GlobalAction` enum in `src/tui/keymap.rs` (split into `tui_pane::GlobalAction` + `AppGlobalAction` in Phase 14). **Audit pass:** every binary call site that names `crate::keymap::GlobalAction::*` flips to either the framework dispatch chain (the bulk of `handle_global_key`'s 11-arm match) or — for the pane-context-specific overrides — a reverse-lookup against `app.framework_keymap` plus the matching `tui_pane::GlobalAction` / `AppGlobalAction` variant. Specific call sites the audit covers (non-exhaustive seed list): `src/tui/input.rs::handle_overlay_editor_key` (uses `GlobalAction::OpenEditor` outside `handle_global_key`).
- `src/tui/input.rs::handle_overlay_editor_key` itself (the 22-line wholesaler at `input.rs:463-484` plus its call site at `:125`). Once `AppGlobalAction::OpenEditor` flows through `AppGlobalAction::dispatcher`, the dispatcher reads the focused overlay context (`InputContext::Settings` / `Keymap`) and routes to `open_path_in_editor` directly. Helpers `overlay_editor_target_path` / `open_path_in_editor` survive (consumed by the dispatcher); the wrapper does not.
- `src/tui/input.rs::handle_global_key` fn body, its 11-arm match on `GlobalAction::*`, and the call site that invokes it from `handle_normal_key` / the dispatch entry. Replaced by the framework dispatch chain — framework-owned globals fire through `tui_pane::GlobalAction::dispatcher`, app-owned globals fire through `AppGlobalAction::dispatcher` (per the **Wire dispatchers** sub-section above).
- The three overlay short-circuits at `src/tui/input.rs:126-137` (`if app.overlays.is_keymap_open() { keymap_ui::handle_keymap_key(...); return; }`, `if app.overlays.is_finder_open() { finder::handle_finder_key(app, code); return; }`, `if app.overlays.is_settings_open() { settings::handle_settings_key(app, code); return; }`). All three are replaced by the framework overlay dispatch chain: `Framework::overlay()` returns the active overlay id, Settings / Keymap overlay actions resolve through `app.framework_keymap.settings_overlay()` / `keymap_overlay()` before mutating the pane, and Finder routes through `Mode::TextInput(finder_keys)` from `FinderPane::mode()`. The underlying helper bodies in `src/tui/finder.rs`, `src/tui/keymap_ui.rs`, and `src/tui/settings.rs` are partially reused by the binary-injected `Mode::TextInput` handlers (per Phase 17) and by the framework-routed dispatcher bodies; legacy `handle_settings_adjust_key` / `handle_settings_edit_key` are gone (Phase 15 retrospective), and the `handle_settings_key` / `handle_keymap_key` entry points themselves either delete entirely or shrink to extracted helper functions consumed by the framework path.
- The `PaneBehavior::Toasts => handle_toast_key(app, &normalized);` arm at `src/tui/input.rs:148` (the call site, not the function body — the body deletes in Phase 26 alongside `app.toasts`). Replaced by the Phase-15-deferred (b) Toasts focus gate chain in `handle_key_event`.
- Every read through `app.keymap.current().global` — including the `action_for`, `key_for`, `display_key_for`, and `display_keys_for` call sites scattered across `src/tui/input.rs`, `src/tui/shortcuts.rs`, and any keymap-overlay UI. With `ResolvedKeymap.global` deleted (next bullet), these accessors have no backing storage. Each call site flips to a framework-keymap reverse-lookup (`app.framework_keymap.framework_globals()` for `tui_pane::GlobalAction`, the `AppGlobalAction` scope getter for the app-owned variants) or is removed entirely if the dispatcher swap obviates it.
- `ResolvedKeymap.global: ScopeMap<GlobalAction>` field on `src/keymap.rs::ResolvedKeymap` — the parallel-path invariant ends here. The framework keymap's globals registries (`framework_globals` for `tui_pane::GlobalAction`, `app_globals` for `AppGlobalAction`) are the sole source of truth post-cutover.
- The local `macro_rules! action_enum` block at `src/keymap.rs:216-250`. By Phase 19, `GlobalAction` is the only remaining caller (every other binary action enum migrated to `tui_pane::action_enum!` during Phases 14.2–14.6). Deleting `GlobalAction` removes the macro's last consumer; the macro itself deletes in the same pass.
- `Overlays::should_quit` accessor and the `should_quit` flag on `Overlays` — replaced by `framework.quit_requested()`.
- The seven static constants (`NAV`, `ARROWS_EXPAND`, `ARROWS_TOGGLE`, `TAB_PANE`, `ESC_CANCEL`, `ESC_CLOSE`, `EXPAND_COLLAPSE_ALL`) and all their call sites.
- `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups` per-context helpers.
- Threaded `enter_action` / `is_rust` / `clear_lint_action` / `terminal_command_configured` / `selected_project_is_deleted` parameters.
- The dead `enter_action` arm in `project_list_groups`.
- The CiRuns `Some("fetch")` label at EOL (the bar bug).
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`. **Note:** the deletion list applies only to the pre-refactor binary types. New `tui_pane` `const fn`s (`BarSlot::primary`, framework `const fn` getters) are kept — do not run a careless `s/const fn/fn/` sweep.
- `src/tui/render.rs::shortcut_spans` and `shortcut_display_width` (the `&[Shortcut]` → `Vec<Span>` flattener and its width helper). `tui_pane::StatusBar` ships pre-styled `Vec<Span<'static>>` per region (Phase 14's `BarPalette` answer), so these helpers have no consumer left after the call-site swap. Phase 26 closeout also moves the outer status-line layout, uptime indicator, scanning-progress span, and global-strip composition into `tui_pane::render_status_line(...)`. The binary's `cargo_port_bar_palette()` constructor stays as app theme data, not bar rendering logic.
- The file-head `#![allow(dead_code, reason = "Phase 14.x landing; later chunks construct these")]` on `src/tui/framework_keymap.rs` — once Phase 19 wires every dispatcher fn to its real handler and routes input through the framework keymap, every variant becomes constructed and the allow is no longer needed.
- The inherent-method facade blocks on `PackageAction` / `GitAction` / `TargetsAction` / `CiRunsAction` / `LintsAction` / `ProjectListAction` (each pane's `pub const ALL` + `pub fn toml_key` / `description` / `from_toml_key` forwarders to the `tui_pane::Action` trait impl, added during 14.2–14.4 to keep legacy call sites compiling) moved out of the Phase 19 deletion list. Phase 22 deletes them after Phase 20 removes the keymap model dependency and Phase 21 moves overlay input/render to the framework. **Excluded from the facade cleanup:** `LangAction` / `CpuAction` (defined locally in `src/tui/framework_keymap.rs` during 14.4a) and `OutputAction` / `FinderAction` (defined in `src/keymap.rs` during 14.5–14.6) all ship without a facade — none of them have facade-removal work.
- **Import-path migration in `src/tui/panes/actions.rs`.** Today `actions.rs:13-18` imports `CiRunsAction`, `GitAction`, `KeyBind`, `LintsAction`, `PackageAction`, `TargetsAction` from `crate::keymap` (the legacy re-export hub). Phase 22 flips these imports to `tui_pane::*` direct where possible (per the flat-namespace rule in Phase 13's "Public API surface" section). `KeyBind` flips to `tui_pane::KeyBind`; the action enums flip to wherever the type-consolidation decision (see Phase 19 `KeyBind` consolidation note) lands them. Same flip applies to the parallel imports in `src/tui/input.rs` and `src/tui/framework_keymap.rs`.

Moved to Phase 20: re-bless `tests/assets/default-keymap.toml` against the framework's defaults. `expand_row` / `collapse_row` flip from `Shift+Right` / `Shift+Left` (the legacy `ResolvedKeymap::defaults` values picked in 14.4c so the TOML round-trip passes the legacy `is_navigation_reserved` check) to plain `Right` / `Left` once the keymap model cleanup retires `ResolvedKeymap`. The asset stays alive because it is the golden file the regenerated default-keymap template is checked against; only its contents change.
- The temporary `build_for_app` shim on `src/tui/framework_keymap.rs` (added in 14.2 so `construct.rs` could build against `app.framework` without leaking broader internals) — already removed by Phase 14.8 along with the two-step initializer collapse. Phase 19 verifies it is gone; no new Phase 19 work expected.

**No `action_enum!` form-normalization sweep.** Phase 19 does not flip surviving 3-positional `tui_pane::action_enum!` invocations to 2-positional. Any 3-positional invocation still in the tree at this point is correct: either bar_label genuinely differs from toml_key (the explicit form is the right choice), or the variant happens to coincide and converting is pure churn. Leave them.
- The legacy `app.focus: Focus` field on `App` and the `self.focus.set(id.to_legacy())` mirror line inside `App::set_focus` move to Phase 22. After that deletion, the override body becomes `self.framework.set_focused(focus);` — which is **identical to the `AppContext::set_focus` default impl** at `tui_pane/src/app_context.rs:42-44`. Phase 22 deletes the override entirely (the trait default takes over) unless Phase 23's funnel test depends on the override surface itself for an inert observation seam. Default deletion preferred. Every render / dispatch path that previously read `app.focus.current()` reads `app.framework().focused()` instead.
- **Residual focus-writer migration.** Before Phase 23's funnel test can mean what it claims, Phase 22 audits and classifies every direct legacy focus writer. Current inventory: 16 production direct writers (`finder.rs:653`, `interaction.rs:29,45, input.rs:118,353,359`, `app/mod.rs:333,653,835,840,855,860,908`, `panes/actions.rs:386`, `app/async_tasks/tree.rs:36,66`) plus the temporary `framework_keymap.rs` mirror noted above. Each site lands in a delete / migrate bucket: app-pane targets route through `app.set_focus(FocusedPane::App(...))`, Toasts targets route through `app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts))`, and dead pre-cutover branches delete with the legacy path. This is the writer checklist only; the broader `app.focus` field deletion still has to migrate reads, overlay-return semantics, and tests.

After Phase 19, `shortcuts.rs` contains only legacy types pending removal (or is deleted entirely if all callers have flipped to `Shortcuts::visibility` / `Shortcuts::state`). The `InputContext` enum is deleted; tests under `src/tui/app/tests/` referencing it migrate to `app.framework().focused()`-based lookups in this phase.

**`handle_finder_key` after the Phase-15-deferred (a) `finder_keys` body swap.** Through Phase 14.6 / Phase 15, `framework_keymap.rs::finder_keys` delegated wholesale to `src/tui/finder.rs::handle_finder_key`. Phase 19's deferred-(a) work changes that: `finder_keys` becomes a `dispatch_app_pane(FinderPane::APP_PANE_ID, ...)` call with a fallback for the non-`FinderAction` arms (Char append, Backspace delete, Up / Down / Home / End result-list nav). Two outcomes are valid:
- If `FinderPane::dispatcher` and the `finder_keys` fallback together still reuse `handle_finder_key`'s extracted helpers (e.g. `finder::query_append_char`, `finder::result_nav`, `finder::activate`, `finder::cancel`), then `handle_finder_key` itself can delete and its arm bodies move to those helpers, called from the dispatcher / fallback.
- If `FinderPane::dispatcher` and the `finder_keys` fallback inline the bodies directly (no helpers extracted), then `handle_finder_key` deletes outright as dead code.
Either way, the legacy short-circuit at `src/tui/input.rs:130-132` (the `if app.overlays.is_finder_open()` branch) is gone (listed in the `Delete:` block above), and `handle_finder_key` no longer survives as a single-call wholesaler. Phase 19's implementer should extract whatever finder helpers the new path needs and delete dead code in the same pass.

Hoist `make_app` from `tests/mod.rs` to `src/tui/tui_test_support.rs` (`pub(super) fn make_app`); declare `#[cfg(test)] mod tui_test_support;` in `src/tui/mod.rs`.

**Relocate framework-only tests from the binary to `tui_pane`.** Walk every `#[test]` and `#[cfg(test)] mod tests` in `src/tui/keymap.rs`, `src/tui/keymap_state.rs`, and any Phase 14-onwards test under `src/tui/` that exercises only `tui_pane` types through cargo-port's `App`. Concretely: keymap TOML loading, scope dispatch through `Keymap::scope_for`, vim-mode application by the builder, default-binding round-trips, action `from_toml_key`/`bar_label` lookups. Move each to `tui_pane/tests/` (one file per concern, e.g. `tests/keymap_loader.rs`, `tests/scope_dispatch.rs`, `tests/vim_application.rs`) against a **minimal mock context** — a small `MockApp` struct matching the one in `tui_pane/src/keymap/shortcuts.rs::tests` (a `Framework<MockApp>` field plus a tiny `MockPaneId` enum). Tests that genuinely depend on `App` state (focus transitions, toast manager, watcher integration) stay in the binary. Outcome: the framework's behavior tests live with the framework, the binary tests only what is binary-specific.

### Retrospective

**What worked:**
- `src/tui/input.rs::handle_key_event` now routes the live non-overlay dispatch path through `app.framework_keymap`: framework globals, app globals, focused app-pane scopes, then navigation fallback.
- The status bar swap is small at the boundary: `src/tui/render.rs::render_status_bar` now asks `tui_pane::render_status_bar(...)` for pre-styled nav / pane-action / global spans while keeping the binary's layout wrapper and `cargo_port_bar_palette()`.
- Wrapping `tui_pane::Keymap<App>` in `Rc` let dispatch clone the keymap before passing `&mut App` into dispatcher bodies, avoiding borrow conflicts without widening the framework API.

**What deviated from the plan:**
- The live cutover kept the legacy `app.focus` field, `InputContext`, `ResolvedKeymap.global`, the local `action_enum!` macro, and the action-enum facade impls. They still back cargo-port's render focus state, overlay-return semantics, the keymap viewer/editor model, and existing tests.
- Settings / Keymap / Finder overlay input still enters through the binary overlay handlers. Framework Settings / Keymap overlay state is mirrored when framework globals open or close those overlays, but cargo-port rendering still reads `app.overlays`.
- `handle_overlay_editor_key` remains as an overlay-specific `OpenEditor` preflight because overlay handlers still intercept before normal global dispatch.
- `tests/assets/default-keymap.toml` was not re-blessed; default-template generation still flows through the legacy `ResolvedKeymap` model.

**Surprises:**
- Once framework dispatch became authoritative, keymap hot reload and the keymap UI save path had to rebuild `app.framework_keymap`; otherwise user rebinds updated the displayed model but not the live dispatcher.
- Framework `OpenSettings` / `OpenKeymap` alone did not display cargo-port's existing popups. The cutover needs a temporary mirror between framework overlay state and legacy `Overlays` until the overlay render path moves fully to `tui_pane`.

**Implications for remaining phases:**
- Phase 20 migrates the keymap viewer/editor off the legacy `ResolvedKeymap` model and proves framework-keymap scopes survive UI save and external reload.
- Phase 21 moves Settings / Keymap input and rendering to framework-owned overlay state, and routes the app-owned Finder pane through the framework keymap / text-input gate.
- Phase 22 removes the remaining focus/input compatibility layer (`app.focus` direct writers, `InputContext`, action-enum facade leftovers).
- Phase 23 writes the final regression suite against the cleaned-up production path. Focus-funnel and production-overlay assertions belong there, after Phases 20-22 have landed.

### Phase 20 — Legacy keymap model cleanup

Phase 20 starts the closeout by migrating the keymap viewer/editor off `ResolvedKeymap`, not just `ResolvedKeymap.global`. The UI rows come from framework keymap metadata for every editable scope: framework globals, app globals, navigation, app panes, Finder, Output, Settings, and Keymap overlays.

**Scope:**
- The keymap UI save path preserves/writes framework-keymap scopes instead of regenerating TOML from `ResolvedKeymap::default_toml_from(...)`; otherwise non-legacy scopes (`[finder]`, `[output]`, `[navigation]`, `[settings]`, `[keymap]`) disappear on save.
- Delete `ResolvedKeymap.global`, the legacy `crate::keymap::GlobalAction`, and the local `action_enum!` macro once the viewer/editor no longer depend on them.
- Re-bless `tests/assets/default-keymap.toml` from the framework defaults after the legacy generator is gone, including the ProjectList `expand_row` / `collapse_row` default-key flip to the framework's authoritative bindings if still applicable.

**Acceptance tests:**
- Keymap UI save path for framework-keymap scopes: edit a non-legacy scope through the keymap UI (for example `OutputAction::Cancel` or `FinderAction::Cancel`), save it, rebuild/reload the framework keymap, and assert production dispatch sees the new binding.
- External file reload for the same class of framework-keymap scope so UI-save and watcher reload both prove non-legacy scopes survive.
- Default-keymap asset generation includes framework-keymap scopes and does not drop `[finder]`, `[output]`, `[navigation]`, `[settings]`, or `[keymap]`.

### Retrospective

**Shipped:**
- Keymap UI rows now read from `app.framework_keymap` for framework globals, app globals, navigation, all app pane scopes, Finder, Output, Settings, and Keymap overlays. The legacy `ResolvedKeymap` model no longer drives the viewer/editor.
- Keymap UI saves write the full framework-keymap TOML surface and rebuild `app.framework_keymap` after saving. Startup/default backfill and external reload also preserve framework-keymap scopes instead of regenerating from `ResolvedKeymap::default_toml_from(...)`.
- `ResolvedKeymap.global`, the legacy `crate::keymap::GlobalAction`, and the local `macro_rules! action_enum` block are gone. The remaining legacy `ResolvedKeymap` only carries pane scopes that still have compatibility readers.
- `tui_pane::Keymap` gained `keys_for_toml_key` so save generation can preserve TOML arrays such as `cancel = ["Esc", "q"]` rather than flattening to the primary key.
- `tests/assets/default-keymap.toml` is now generated from the framework defaults and includes `[global]`, `[navigation]`, all app pane scopes, `[finder]`, `[output]`, `[settings]`, and `[keymap]`.

**Surprises:**
- The golden-template test must override the keymap path to a temp file. Otherwise it can accidentally read the developer's real `~/.config/cargo-port/keymap.toml` and compare user rebinds against the default asset.
- Preserving framework-keymap scopes was not enough; the save path also had to preserve multi-key bindings. The existing structural Output cancel test caught this when `["Esc", "q"]` collapsed to `"Esc"` during save/rebuild.

**Verification:**
- `cargo check --workspace --all-targets -q`
- `cargo mend --workspace --all-targets`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo nextest run` — 625 passed
- `cargo install --path .`

**Remaining-phase closeout gate.** Starting with Phase 20.1 and continuing through Phase 26, every phase closes only after:
1. Run the `/clippy` skill. That includes the Rust style-guide load, `cargo mend --workspace --all-targets`, style review, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo +nightly fmt --all` per the skill.
2. Run all tests with `cargo nextest run --workspace`.
3. Install the local binary with `cargo install --path .` so the user can run a smoke test against the just-built code.
4. Record the command results in the phase retrospective. If any command cannot run, the phase does not close as verified; record the blocker instead.

### Phase 20.1 — Framework tab traversal policy

Phase 20.1 fixes the Phase 19 focus-cycle regression before overlay ownership and focus-compatibility cleanup continue. The framework owns `GlobalAction::NextPane` / `PrevPane` traversal mechanics, but registration order is not tab order and it is not availability. Apps declare stable tab stops at registration time and provide live tabbability predicates; the framework builds the current cycle from that metadata on each traversal.

**Why this lands here:** Phase 20 is already the active keymap-model cleanup and should not absorb a focus API redesign. Phase 21's overlay tests depend on `Tab` already resolving correctly, Phase 22 deletes legacy focus scaffolding, and Phase 23 is final regression coverage. Therefore the traversal contract lands as a small corrective phase between Phase 20 and Phase 21.

**Library contract:**
- Add a `TabStop<Ctx>` registration value in `tui_pane`, re-exported at the crate root. Shape:

  ```rust
  pub enum TabOrder {
      Registration,
      Explicit(i16),
      Never,
  }

  pub struct TabStop<Ctx: AppContext> {
      order:        TabOrder,
      is_tabbable: fn(&Ctx) -> bool,
  }
  ```

- Add constructors on `TabStop`: `registration_order()`, `ordered(order, is_tabbable)`, `always(order)`, and `never()`. `registration_order()` preserves existing test/mock behavior: registered panes are tabbable in registration order unless they opt into explicit ordering or `never`.
- Extend `Pane<Ctx>` with `fn tab_stop() -> TabStop<Ctx>`, defaulting to `TabStop::registration_order()`. Keep `Pane::mode()` unchanged.
- `KeymapBuilder::insert_pane` records `P::tab_stop()` alongside `P::APP_PANE_ID` and `P::mode()`. `build_into(&mut Framework<Ctx>)` writes all three pieces into the framework registry.
- `Framework<Ctx>` stores tab metadata separately from `pane_order`. `pane_order()` remains registration-order metadata for existing callers and tests; it is no longer the focus-cycle source of truth.
- Add a framework-internal live-cycle helper that sorts app panes by `TabOrder::Explicit(n)` with registration-order tie-breaks, keeps `TabOrder::Registration` panes in registration order after explicit panes, drops `TabOrder::Never`, filters every remaining pane through `is_tabbable(ctx)`, then appends `FocusedPane::Framework(FrameworkFocusId::Toasts)` when toasts are active.
- Rewrite `framework::dispatch::focus_cycle`, `focus_step`, and `reconcile_focus_after_toast_change` to use that live-cycle helper. Reconciliation after the focused toast stack empties moves to the first live app tab stop, not `pane_order().first()`.
- If the currently focused pane is absent from the live cycle because it is no longer tabbable, `NextPane` moves to the first live entry and `PrevPane` moves to the last live entry. This preserves today's fallback semantics while preventing unavailable panes from remaining reachable.
- Preserve the existing focused-Toasts behavior: when `NextPane` / `PrevPane` is rebound, the same keymap-resolved action still drives toast scroll-before-advance; entering Toasts resets the viewport to first/last based on direction; exhausting Toasts returns to the app cycle.

**Cargo-port registration policy:**
- Register normal app panes with explicit row-major tab order derived from the visual layout:
  `ProjectList`, `Package`, `Git`, `Lang`, `Cpu`, `Targets`, `Lints`, `CiRuns`, `Output`.
- Register `FinderPane` as `TabStop::never()`. Finder is an app-defined modal overlay scope, not a normal tab-cycle pane.
- Wire each normal pane's `is_tabbable` predicate to the existing `App::is_pane_tabbable` policy. Keep that policy app-owned because it depends on selected project state, loaded content, current output state, lint/CI data, and whether panes are visually obscured.
- Output remains app policy. When `example_output` is present, `Lints` and `CiRuns` report untabbable and `Output` reports tabbable; when output is absent, `Output` reports untabbable and diagnostics panes decide from their own content.
- Do not intercept `Tab` / `Shift+Tab` in `src/tui/input.rs`, do not resurrect production `focus_next_pane` / `focus_previous_pane`, and do not add an `AppContext::focus_cycle` override. The framework dispatcher remains the production path.

**Acceptance tests:**
- `tui_pane` tests: explicit `TabOrder` beats registration order; `TabOrder::Never` panes are excluded; an `is_tabbable` predicate returning `false` skips an otherwise ordered pane; `PrevPane` walks the same filtered cycle in reverse; `reconcile_focus_after_toast_change` uses the first live app tab stop.
- `tui_pane` stale-focus tests: when the current app pane has dropped out of the live cycle, `NextPane` chooses the first live entry and `PrevPane` chooses the last live entry.
- `tui_pane` Toasts tests: active Toasts append after app panes, entering Toasts resets the viewport, and scroll-before-advance still follows the rebound `GlobalAction::NextPane` / `PrevPane` actions.
- Cargo-port production-path tests through `handle_key_event`: from `Package`, `Tab` lands on `Git` when `Git` is available and `Lang` is unavailable; repeated `Tab` never lands on unavailable panes; `Shift+Tab` skips unavailable panes in reverse; output active makes covered diagnostics panes untabbable and makes `Output` reachable.
- Rebinding `GlobalAction::NextPane` to a non-Tab key still uses the same framework-managed, app-filtered tab cycle.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- `TabStop<Ctx>` stayed a small registration-time value: `KeymapBuilder::insert_pane` records it beside each pane id and mode query, and `Framework<Ctx>` computes the live cycle only when traversal fires.
- Reusing cargo-port's existing `App::is_pane_tabbable` predicates kept output/diagnostics behavior app-owned while letting `tui_pane` own `GlobalAction::NextPane` / `PrevPane` mechanics.
- Toasts already had `try_consume_cycle_step`, `reset_to_first`, and `reset_to_last`; Phase 20.1 only had to call them from the framework global dispatcher.

**What deviated from the plan:**
- The cargo-port "repeated Tab" and reverse-Tab assertions were narrowed to "Lang unavailable" instead of "Lang/Cpu unavailable" because the test fixture has valid CPU data. CPU remains a reachable pane, so the regression is that unavailable Lang is skipped.
- The public `TabStop<Ctx>` fields stayed private; the phase only needs public constructors plus the `Pane::tab_stop()` hook.

**Surprises:**
- `#[derive(Clone, Copy)]` on `TabStop<Ctx>` introduced an unwanted generic bound, so `TabStop` uses manual `Copy` / `Clone` impls.
- `cargo nextest run` covered the root package only in this package-plus-workspace layout; `cargo nextest run --workspace` is needed to include `tui_pane`.

**Implications for remaining phases:**
- Phase 21 can assume `Tab` / `Shift+Tab` already route through framework globals and the app-filtered tab cycle; overlay work should not add another Tab interception path.
- Phase 22's focus cleanup can treat `Framework::live_focus_cycle` and `App::set_focus` as the production focus-write path for traversal.

**Verification:**
- `cargo check --workspace --all-targets -q`
- `cargo mend --workspace --all-targets` — no findings
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo nextest run` — 630 passed
- `cargo nextest run --workspace` — 855 passed
- `cargo install --path .`

### Phase 21 — Overlay input/render ownership

Phase 21 removes the production overlay short-circuits that Phase 19 left in place. Settings and Keymap route through framework-owned overlay scopes and their text-input handlers; Finder remains an app-owned modal pane (`FinderPane`, `TabStop::never()`) whose keys route through the framework keymap and its `Mode::TextInput` handler. Rendering reads framework overlay state for Settings / Keymap and the app-owned Finder overlay state for Finder.

**Scope:**
- Remove the `src/tui/input.rs` short-circuits that call `keymap_ui::handle_keymap_key`, `finder::handle_finder_key`, and `settings::handle_settings_key` before the framework dispatch chain.
- Do not add Finder to `FrameworkOverlayId` in this phase. Finder is not a framework overlay; Phase 21 changes routing, not ownership.
- Preserve the `Mode::TextInput` dispatch gate: once the focused text-input path handles or rejects a key, do not fall through to globals or navigation. Settings / Keymap enforce this through framework overlay dispatch; Finder enforces it through its app-owned `FinderPane` text handler.
- Move any surviving legacy handler bodies behind framework pane methods, or delete them once no caller remains.
- Remove the temporary mirror that kept framework `OpenSettings` / `OpenKeymap` in sync with cargo-port's legacy `Overlays` just to display the old popups.
- Delete legacy overlay-return state that is no longer needed, or migrate the return target into the framework focus model.
- Remove or reroute `handle_overlay_editor_key` once overlay input no longer intercepts before normal global dispatch.

**Acceptance tests:**
- Rebinding `FinderAction::Cancel` to `'q'` closes Finder through production `handle_key_event`; `'k'` typed in Finder inserts `'k'` even with vim mode on.
- Rebinding `FinderAction::Activate` to `Tab` while Finder is open fires Activate, NOT `GlobalAction::NextPane`.
- Binding any action to `Up` while `KeymapPane::EditState` is `Awaiting` produces a "reserved for navigation" rejection through production `handle_key_event`.
- Settings in Editing mode: Esc cancels edit; `example_output` is not cleared.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- Settings and Keymap production input now routes through the framework overlay scopes and `Mode::TextInput` handlers. Their popup rendering reads `FrameworkOverlayId::Settings` / `FrameworkOverlayId::Keymap` instead of cargo-port's legacy overlay mirror.
- Finder stayed app-owned, but its production keys now enter through the framework keymap and `FinderPane::mode()` text-input handler. That kept Finder out of `FrameworkOverlayId` while removing the old overlay short-circuit.
- The production tests cover the modal precedence points that mattered for the cutover: Finder Cancel rebind, Finder vim `k` text entry, Finder Tab/Activate beating global NextPane, Keymap reserved-navigation rejection, and Settings Esc not clearing `example_output`.

**What deviated from the plan:**
- The framework panes for Settings / Keymap needed small public state-transition helpers (`enter_editing`, `enter_browse`, `enter_awaiting`) so cargo-port can keep its existing editor buffers while the framework owns overlay routing.
- `EditState::Conflict` in the framework Keymap pane is test-only for now. The closeout style review rejected keeping a production `#[allow(dead_code)]`, so the variant and match arm moved behind `#[cfg(test)]`.

**Surprises:**
- Removing the framework-to-legacy overlay mirror exposed test setup that still opened Settings through `Overlays::open_settings()`. Those tests now open the framework overlay path directly.
- Finder's production route could delete `handle_finder_key` outright once the fallback text/list behavior lived in the `finder_keys` text-input handler.

**Implications for remaining phases:**
- Phase 22 can treat Settings / Keymap overlay input and render gating as framework-owned. Remaining cleanup should focus on direct focus writers, `InputContext`, and action-enum facade leftovers.
- Finder remains app-owned unless a later phase explicitly changes that ownership; Phase 22 should not move Finder into `FrameworkOverlayId` as part of focus cleanup.

**Verification:**
- Rust style guide loaded: 45 files, 1608 lines.
- `cargo mend --workspace --all-targets` - no findings.
- Style review of additions completed; the one violation was fixed by making `EditState::Conflict` test-only instead of allowing dead code.
- Banned-word scan over additions passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo nextest run --workspace` - 858 passed, 0 skipped.
- `cargo install --path .`

### Phase 21 Review

- Phase 22: clarified that focus cleanup migrates production focus reads as well as direct writers before deleting `app.focus`.
- Phase 22: recorded Finder's app-owned return target replacement on `Overlays` before deleting legacy overlay-return state.
- Phase 23: collapsed duplicate modal/text-input tests already shipped in Phase 21 into baseline coverage, keeping only cleanup-path deltas.
- Phase 23: recorded the Settings/Keymap browse-mode `OpenEditor` exception while keeping `Mode::TextInput` as the hard suppression boundary.
- Phase 24: clarified that focused Toasts dispatch checks `ToastsAction` before globals, with globals reached only when the toast action is unhandled.
- Phase 25: clarified the existing SettingsPane ownership plan: generic settings rows, rendering, editing, validation display, and commit routing move into `tui_pane`.

### Phase 22 — Focus and input compatibility cleanup

Phase 22 deletes the remaining focus/input compatibility layer after the keymap model and overlay path have moved to the framework.

**Scope:**
- Migrate every remaining production `app.focus.set(...)` / `self.focus.set(...)` writer through `app.set_focus(FocusedPane::App(...))` or `app.set_focus(FocusedPane::Framework(...))`.
- Migrate focus reads before deleting the field. Every production caller of `app.focus.current()`, `app.focus.is(...)`, `app.focus.behavior()`, and related query helpers moves to `app.framework.focused()` / framework focus queries, or is documented as a temporary survivor with a deletion owner. `handle_key_event` must dispatch from `app.framework.focused()` directly; do not rebuild framework focus from `app.focus.current()` after this phase.
- Replace Finder's legacy overlay-return state before deleting `app.focus`. Finder remains app-owned: store `finder_return: Option<FocusedPane<AppPaneId>>` on `Overlays` beside `FinderMode`, set it from `app.framework.focused()` in `open_finder`, focus Finder with `app.set_focus(FocusedPane::App(AppPaneId::Finder))`, and close Finder by taking the stored return target and calling `app.set_focus(return_target)` with `ProjectList` as the fallback. Delete the `app.focus.open_overlay(PaneId::Finder)` / `app.focus.close_overlay()` dependency.
- Mouse/click focus routing is part of this migration. Top-level mouse routing moves to the framework boundary: app render/hit-test code supplies pane-local regions and domain targets; the framework maps the click to `FocusedPane` and mutates focus only through `app.set_focus(FocusedPane::App(...))`. Pane-local row/domain hit handling stays app-owned. Direct `app.focus.set(...)` writes from `handle_mouse_click` / interaction handling delete in this phase.
- After cleanup, `rg '(app|self)\.focus\.set\(' src` returns zero production hits; any test-only helper survivor must be named explicitly.
- Delete the broader `app.focus` field if no reads remain. If a read must survive temporarily, document it as a render/query survivor with a concrete deletion owner before this phase closes.
- Delete `InputContext` and action-enum facade leftovers once all render/input callers read the framework focus/keymap state directly.

**Acceptance tests:**
- A test impl that overrides `set_focus` to count calls observes every framework-originated focus change: NextPane, PrevPane, focused Toasts, mouse/click app-pane focus, and return-from-overlay. `OpenKeymap` / `OpenSettings` are overlay-state changes, not focus writes; assert `framework.overlay()` changes and the focus-write counter does not increment.
- `rg 'InputContext' src` returns zero production hits.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- `app.focus` and the legacy focus modules deleted cleanly once reads moved to `app.framework.focused()` / `App` focus helpers.
- `InputContext` and `src/tui/shortcuts.rs` had no production survivors after the Phase 21 overlay cleanup.

**What deviated from the plan:**
- `AppContext::set_focus` stayed as an override because it now records `visited_panes` in addition to forwarding to `framework.set_focused(...)`; the trait default is not equivalent.
- Finder return state moved to `Overlays::finder_return: Option<FocusedPane<AppPaneId>>`, with `ProjectList` as the fallback when no return target is recorded.

**Surprises:**
- Removing the action-enum facades was smaller than the plan's early notes implied: `OutputAction` / `FinderAction` never had facade impls, and the remaining legacy keymap call sites could use the `tui_pane::Action` trait generically.
- The only full-suite failure seen during closeout was the existing timing-sensitive `handle_project_discovered_does_not_allocate_per_comparison` threshold; the later rerun was blocked by `sccache: Operation not permitted` after a sandboxed Rust command attempt.

**Implications for remaining phases:**
- Phase 23's focus-funnel regression should assert the surviving `AppContext::set_focus` override as the production funnel, not expect the override to disappear.
- Phase 23 can treat `app.focus`, `InputContext`, and action-enum facade compatibility as gone; remaining tests should target the framework focus/keymap path directly.

### Phase 22 Review

- Phase 23: narrowed cleanup-path regression coverage to the remaining focus/finder/mouse deltas instead of duplicating Phase 20.1 tab-order and Phase 21 overlay-input tests.
- Phase 23: added cargo-port's removed-action TOML migration before framework `load_toml`, with acceptance coverage for legacy `[project_list] open_editor` / `rescan` keys and deletion of `src/keymap.rs::is_legacy_removed_action`.
- Phase 23: added closeout cleanup for stale `src/tui/shortcuts.rs` / `shortcut_spans` references now that the module is gone.
- Phase 24: made focused-Toasts dispatch ordering explicit: once `ToastsAction::Activate` exists, focused Toasts get first claim before framework/app globals.
- Phase 25: named deletion of the remaining Settings/Keymap binary mirror state and `clear_legacy_framework_overlay_state`.

### Phase 23 — Regression tests

Phase 23 writes the final regression suite against the cleaned-up production path from Phases 20-22.

**Test-mod allow pattern.** Phase 23 adds many new tests across `src/tui/app/tests/` and (per the relocation bullet at the end of this section) under `tui_pane/tests/`. Every test module follows the established pattern shipped during 14.2 / 14.7 / 14.8: a single `#[cfg(test)] #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, reason = "tests should panic on unexpected values")] mod tests { ... }` header (see `src/tui/app/mod.rs:103-112`, `src/tui/framework_keymap.rs:667-672`, `src/tui/interaction.rs:142-148`). Per-test `#[allow]` attributes are not added; the module-level block covers them.

- **Snapshot truth — global-strip total order under default bindings.** With Phase 19 having wired the framework dispatcher and Phases 20-22 having removed the legacy mirrors, the per-focused-pane `bar.global` snapshot below locks this order: `quit, restart, keymap, settings, dismiss, find, editor, terminal, rescan`. The framework walks `tui_pane::GlobalAction` first (`render_framework_globals_slots`, with `NextPane`/`PrevPane` filtered out by `bar/global_region.rs:42` since they render in the pane-cycle row) then `AppGlobalAction` second (`render_order() == AppGlobalAction::ALL`). This is the total-order ground truth for every Phase 23 snapshot under default bindings; rebind snapshots assert deltas against this baseline.

Bar-on-rebind:

**Acceptance baseline for every "Rebinding X to Y" test below.** Phase 18 already wired framework-keymap TOML loading through `App::new`, and Phase 17 shipped shared-`[global]`-table coordination. Every rebind test below uses that production loader path; otherwise the test exercises built-in defaults rather than user rebinding and is invalid as a cutover-regression test.

**Production-loader-only rebind tests.** Phase 18 deleted the temporary `build_framework_keymap_with_toml(...)` helper and wired `.load_toml(keymap_path)` into the production builder. Phase 23 rebind tests use the production loader path only; settings/keymap overlay rebind tests also require Phase 18's builder chain to call `register_settings_overlay()` / `register_keymap_overlay()` so the resolved overlay scopes carry user TOML.

**Modal/overlay dispatch scope after cleanup.** Phase 21 already shipped production `handle_key_event` coverage for Finder Cancel rebind, Finder vim `k`, Finder Tab/Activate precedence, Keymap reserved-navigation rejection, and Settings Esc edit cancellation. Phase 20.1 already shipped tab-order coverage. Phase 23 adds only cleanup-path deltas that depend on Phase 22's focus/input compatibility deletion and the broad snapshot/rebind suite; direct framework-scope tests remain useful for narrow loader/bar assertions.

- Rebinding each `*Action::Activate` (`Package`, `Git`, `Targets`, `CiRuns`, `Lints`) updates that pane's bar.
- Rebinding `NavigationAction::Up` / `Down` / `Left` / `Right` updates the `↑/↓` nav row in every base-pane bar that uses it.
- `keymap.navigation::<AppNavigation>().expect(...).key_for(NavigationAction::Home).copied() == Some(KeyCode::Home.into())` and the same round-trip for `End` after a default build (locks the Phase 12 `Navigation` trait extension; uses the typed singleton getter on `Keymap`, since the public bar surface in Phase 13 is type-erased and exposes only `render_navigation_slots()`).
- Rebinding `GlobalAction::NextPane` updates the pane-cycle row.
- Rebinding `ProjectListAction::ExpandAll` / `CollapseAll` updates the `+/-` row.
- Rebinding `ProjectListAction::ExpandRow` / `CollapseRow` updates the `←/→ expand` row.
- With `VimMode::Enabled`, `ProjectListAction::ExpandRow`'s bar row shows `→/l` (vim extra merged into the scope by the builder, surfaced through `display_keys_for`); `CollapseRow` shows `←/h`.
- Rebinding `FinderAction::Activate` / `Cancel` updates the finder bar. Do not list `PrevMatch` / `NextMatch` unless the phase explicitly adds those `FinderAction` variants.
- Rebinding `OutputAction::Cancel` updates the output bar.
- Rebinding settings/keymap actions (framework-internal) updates their bars.

Globals + precedence:

- Globals render order matches the framework's render order then `AppGlobals::render_order()`; each slot's bar text comes from `action.bar_label()` (Phase 5's `Action::bar_label`), not a `Globals` trait method.
- CiRuns Activate at EOL renders no Enter row.
- `key_for(NavigationAction::Up) == KeyBind::from(KeyCode::Up)` even when vim mode is on.
- Rebinding `GlobalAction::Quit` to `q` keeps `q` quitting from any pane (global beats unbound).
- Rebinding `GlobalAction::NextPane` to `j` (vim-off) cycles panes from any base pane.
- Rebinding `ProjectListAction::ExpandRow` makes the pane-scope binding fire instead of `NavigationAction::Right`.
- Rebinding `FinderAction::Activate` to `Tab` while Finder is open fires Activate, NOT `GlobalAction::NextPane`.
- **Tab-cycle coverage note.** Phase 20.1 already covers tab-stop ordering, stale-focus fallback, Toasts scroll-before-advance, and cargo-port production `Tab` / `Shift+Tab` traversal. Phase 23 keeps only broad cleanup-path assertions here, such as user rebinds flowing through the post-cleanup production loader and dispatcher.
- **`AppContext::set_focus` is the single funnel for focus writes.** Phase 22 already removed production direct focus writers. Phase 23 keeps the end-to-end regression test that overrides `set_focus` to count calls and covers the remaining cleanup-path deltas: `PrevPane`, mouse/click app-pane focus, Finder return-to-origin, and focused-Toasts exit back into the app cycle. `OpenKeymap` and `OpenSettings` are overlay-state changes, not focus writes; cover them with separate assertions that `framework.overlay()` becomes `Some(FrameworkOverlayId::Keymap)` / `Some(FrameworkOverlayId::Settings)` and that the focus-write counter does not increment. Do not duplicate Phase 20.1's tab-order assertions or Phase 21's overlay input tests here.

Dispatch parity (per pane, the highest-risk path):

- For each `*Action::Activate` (Package/Git/Targets/CiRuns/Lints): rebind to `'a'`, synthesize an `'a'` key event, assert the pane's free-function dispatcher ran. **Assertion observed via the dispatcher's side effect** (atomic counter, captured `Cell<Option<Action>>`, etc.) — `KeyOutcome::Consumed` only signals "a binding fired"; *which* action ran is observed through the dispatcher itself.
- Rebind `AppGlobalAction::OpenEditor` to `'E'`, synthesize `'E'`, assert `open_editor` dispatched.
- Rebind `GlobalAction::Dismiss` to `Ctrl+D`, synthesize `Ctrl+D`, assert `dismiss` injected closure ran.

Vim/text-input regression:

- Reuse the Phase 21 production tests for Finder text-input precedence, Finder Cancel rebind, Finder Tab/Activate precedence, Keymap reserved-navigation rejection, and Settings Esc edit cancellation as the baseline. Add Phase 23 assertions only where Phase 22's deletion of legacy focus/input compatibility changes the path under test.
- **Framework-side `Mode::TextInput` bar suppression.** Already covered by `tui_pane/src/bar/tests.rs::textinput_mode_suppresses_every_region`; do not add a duplicate Phase 23 test unless the intent is explicitly public-API integration coverage. The cargo-port-side test (`focused_finder_open_bar_suppresses_all_regions` in `src/tui/app/tests/framework_keymap.rs`, landed in 14.6) covers Finder specifically; the `tui_pane` unit test pins the generic rule for any future `TextInput` pane.
- **Policy: no globals fire while focused pane is `Mode::TextInput`.** This is parity with today's legacy short-circuit in `src/tui/input.rs::handle_normal_key`, which gates on `app.overlays.is_finder_open() || app.overlays.is_settings_editing()` *before* `handle_global_key`. The framework's "`Mode::TextInput` suppresses every region" rule at `tui_pane/src/bar/{nav_region.rs:33, pane_action_region.rs:24-25, global_region.rs:31-32}` is bit-for-bit equivalent. **No allow-list this phase.** Any opt-in allow-list (e.g. surfacing `Quit` / `Restart` / `Rescan` while a text-input pane is active) is post-Phase-19 design work — it is new API surface (probably a per-action `survive_text_input` bit on `Globals::Actions` or an equivalent framework escape hatch), not migration parity. Phase 23 ships with the suppression rule, no exceptions. **Acceptance test:** rebind `AppGlobalAction::Rescan` to `Ctrl+r` (its default) and synthesize that key event with focus on the open Finder; assert the rescan dispatcher does **not** fire (observed via the dispatcher's side-effect counter from the `Dispatch parity` block above). Locks the parity guarantee.
- **Settings/Keymap browse-mode `OpenEditor` exception.** The policy is not "globals never run while an overlay is open." `Mode::TextInput` remains a hard suppression boundary, but Settings / Keymap browse mode has one deliberate preflight for `AppGlobalAction::OpenEditor` so `e` can open the active config/keymap file from the overlay. This is not a general text-input allow-list and does not extend to Settings Editing or Keymap Awaiting. Acceptance coverage: browse-mode `OpenEditor` dispatches, and Settings Editing / Keymap Awaiting do not dispatch `OpenEditor` even when rebound.

TOML loader:

- **Cargo-port removed-action TOML migration.** Before any cargo-port call to `tui_pane::KeymapBuilder::load_toml(path)`, run a binary-owned TOML normalization pass over the file contents. This pass rewrites legacy keys that used to live under `[project_list]` into their current global homes: `[project_list] open_editor = X` moves to `[global] open_editor = X`, and `[project_list] rescan = X` moves to `[global] rescan = X`. If the `[global]` key already exists, keep the explicit `[global]` value and drop the stale `[project_list]` key. Write the normalized TOML back only when a stale key was removed or moved. Then feed the normalized file to the framework loader. This implements the earlier TOML-loader rule that `tui_pane` carries no removed-action migration; cargo-port owns its own historical key names.
- `[finder] activate = "Enter"` and `cancel = "Enter"` → `Err(KeymapError::CrossActionCollision)`.
- TOML scope replaces vim+defaults: `[navigation] up = ["PageUp"]` with vim-on → `key_for(Up) == PageUp`, `'k'` not bound.
- Cargo-port config drives framework `VimMode`: `NavigationKeys::ArrowsAndVim` binds navigation `h/j/k/l` and ProjectList vim extras; `NavigationKeys::ArrowsOnly` does not bind those extras. Run this through the production builder path, not a hand-built `KeymapBuilder`, so it catches missed startup/reload wiring.
- Malformed binding propagation: round-trip a malformed binding string through the production TOML overlay loader and assert `Err(KeymapError::InvalidBinding { source, .. })`, with `err.source().is_some()` so the underlying `KeyParseError` is preserved. `KeymapError::KeyParse(_)` remains the unscoped direct-conversion path, not the scoped `load_toml` variant.
- Acceptance coverage for the removed-action migration: a keymap file with `[project_list] open_editor = "E"` / `rescan = "Ctrl+r"` and no corresponding `[global]` entries lets `App::new` build, resolves those bindings through `AppGlobalAction::OpenEditor` / `Rescan`, and rewrites the file with the stale project-list keys removed. A file with both old and new keys keeps the `[global]` value. A truly unknown `[project_list]` key such as `claen = "c"` still errors. After this lands, delete `src/keymap.rs::is_legacy_removed_action`; the local legacy loader no longer ignores removed keys because the migration has already moved or removed them before validation.

A snapshot test per focused-pane context locks in the **new** static-label framework bar — the bar text and span styles produced by `render_status_bar(focused, &app, &framework_keymap, &framework, &cargo_port_bar_palette())` under default bindings. The snapshots are not asserted byte-identical to the pre-refactor dynamic-label bar; Phase 14's `bar_label` collapse (one static literal per variant) deliberately drops row-dependent labels (e.g. `PackageAction::Activate`'s `"URL"`/`"Cargo.toml"` switch) in favor of one short generic label per action. The fixture drives the renderer through `framework.focused_pane_mode(ctx)` and the `AppPaneId`-keyed `Keymap::render_app_pane_bar_slots` (Phase 9 + Phase 13) — never via a typed `P::mode()` call — so each snapshot parameterizes on `FocusedPane`, not on the concrete pane type. The palette is `cargo_port_bar_palette()` (Phase 14); a different palette would diverge on style attributes by design.

**`KeyBind` construction in Phase 19 / Phase 23 tests.** `tui_pane::KeyBind` exposes `From<char>` and `From<KeyCode>` only — no `plain` constructor (the legacy `crate::keymap::KeyBind::plain` does not transfer). Use `KeyBind::from('k')` for character keys and `KeyBind::from(KeyCode::Tab)` for named keys. For modifier-bearing binds, the struct-literal form (`KeyBind { code: …, mods: KeyModifiers::SHIFT }`) is also valid and is what `framework_keymap.rs::PROJECT_LIST_VIM_EXTRAS` uses today (since `From<char>` is not `const`). Phase 14.9's optional `pub const fn KeyBind::from_char(c: char) -> Self` polish would simplify the static-array case; until then, the struct-literal pattern is canonical for `static` initializers.

**Phase 23 closeout:** Run the remaining-phase closeout gate after the regression tests and snapshots land. Also grep for stale references to deleted `src/tui/shortcuts.rs` / `shortcut_spans` and update comments or docs that still describe those helpers as live code.

### Retrospective

**What worked:**
- The removed-action TOML migration is binary-owned and runs before both startup `load_toml` and framework-keymap rebuilds: `src/tui/app/construct.rs` and `src/tui/app/mod.rs` call `keymap::migrate_removed_action_keys_on_disk(...)`.
- Existing `src/tui/app/tests/framework_keymap.rs` coverage was the right home for Phase 23 closeout assertions: global-strip ordering, ProjectList paired rows, and production-loader migration all exercise the real `App::new` / input path.

**What deviated from the plan:**
- The phase extended the existing regression suite instead of introducing new snapshot files. The tests now lock the high-risk bar text/order behavior with focused assertions rather than full fixture snapshots.
- ProjectList paired-row assertions do not assume plain `Left` / `Right`; local or migrated TOML can still bind `Shift+Left` / `Shift+Right`, so the test asserts the paired arrow-row structure and label.

**Surprises:**
- `make_app(...)` without a keymap-path override can read the developer's real keymap file. Exact default-binding tests need a temp keymap path, matching the earlier Phase 20 lesson.
- The final closeout grep found only a live rustdoc reference to `shortcut_spans`; the remaining doc references are historical phase notes or the Phase 23 instruction itself.

**Implications for remaining phases:**
- Later toast/settings phases should not carry compatibility for `src/keymap.rs::is_legacy_removed_action`; it is gone, and removed cargo-port action names are normalized before validation.
- Any remaining exact-default key assertions must override `keymap::keymap_path()` to a temp path before constructing `App`.

**Verification:**
- `cargo +nightly fmt --all`
- `cargo nextest run -p cargo-port framework_keymap` — 47 passed
- `cargo nextest run -p cargo-port legacy_project_list_removed` — 4 passed
- `cargo nextest run` — 636 passed
- `cargo nextest run --workspace` — 861 passed

**Remaining-phase test isolation.** In Phases 24-26, any cargo-port test that constructs `App` and asserts exact default keys, Enter behavior, or bar text must override `keymap::keymap_path()` to a temp path before `App::new`. Otherwise the test can read the developer's real keymap file and assert against user rebinds.

### Phase 23 Review

- Phase 24: recorded the storage boundary — framework toast activation API lands now, but cargo-port production Enter-on-toast acceptance moves to Phase 26 after `app.toasts` migrates into `framework.toasts`.
- Phase 25: superseded the earlier cargo-port `[toasts]` parsing note with a framework-owned `SettingsStore`; toast-setting validation still does not route through `KeymapError` or the keymap overlay loader.
- Phases 24-26: added the temp keymap-path override requirement for cargo-port tests that construct `App` and assert exact default keys, Enter behavior, or bar text.
- Phase 25: added a test-migration note for Settings/Keymap mirror deletion so Phase-23-era overlay assertions move to framework pane state.
- What dissolves: marked `is_legacy_removed_action` as already removed by Phase 23.

### Phase 24 — Toast activation payload (✅ landed)

Phase 24 adds the typed activation payload to the framework's `Toast` so binaries can attach a domain action to each toast that fires on Enter while focused. cargo-port replaces its current `action_path: Option<AbsolutePath>` with `Option<CargoPortToastAction::OpenPath(AbsolutePath)>`; the framework stays generic.

**Storage boundary for this phase.** Phase 24 adds the framework toast-action API and cargo-port's `CargoPortToastAction` type, but it does not make framework toast storage the cargo-port production storage. cargo-port still renders and mutates `app.toasts` until Phase 26. Therefore Phase 24 cargo-port wiring is limited to the associated type / handler and any compile-level boundary tests; production Enter-on-toast behavior is accepted in Phase 26 after the manager migration makes `framework.toasts` the storage users see.

**1. New `AppContext` associated type.** `AppContext` gains `type ToastAction: Clone + 'static;` plus a default handler:

```rust
pub trait AppContext: Sized + 'static {
    type AppPaneId: Copy + Eq + Hash + 'static;
    /// Domain payload attached to a toast and dispatched on Enter while
    /// focused. Apps that do not need toast activation set this to
    /// `NoToastAction` and inherit the default `handle_toast_action`.
    type ToastAction: Clone + 'static;

    fn framework(&self)     -> &Framework<Self>;
    fn framework_mut(&mut self) -> &mut Framework<Self>;
    fn handle_toast_action(&mut self, _action: Self::ToastAction) {
        // default: no-op (action types that never construct cannot reach here).
    }
}

/// Uninhabited filler for apps that have no toast activation. The
/// default `handle_toast_action` is unreachable for this type.
pub enum NoToastAction {}
```

The `'static` bound matches the existing `'static` bound on `AppPaneId`. The default `handle_toast_action` body is `{}` so apps that pick `NoToastAction` (uninhabited) write nothing.

**Implementor checklist.** Adding `type ToastAction` touches every `AppContext` impl, including `tui_pane` unit-test mocks and integration-test contexts. Phase 24 updates cargo-port's `App` to its real `CargoPortToastAction`; every non-activating mock / test context sets `type ToastAction = NoToastAction` and keeps the default `handle_toast_action`.

**2. `Toast<Ctx>` (already generic from Phase 12) gains an `action` field.** The public type signature is unchanged — only the field set grows; `_ctx: PhantomData<fn(&Ctx)>` from Phase 12 is replaced by the `Ctx::ToastAction` reference, which now ties `Ctx` into the struct directly.

```rust
pub struct Toast<Ctx: AppContext> {
    id:     ToastId,
    title:  String,
    body:   String,
    style:  ToastStyle,
    action: Option<Ctx::ToastAction>,
}

impl<Ctx: AppContext> Toasts<Ctx> {
    pub fn push_with_action(
        &mut self,
        title:  impl Into<String>,
        body:   impl Into<String>,
        action: Ctx::ToastAction,
    ) -> ToastId;
}
```

**3. `ToastsAction::Activate` and `ToastCommand<A>`.** Phase 24 adds `Activate` to the previously empty `ToastsAction` enum. Navigation does not flow through `ToastsAction` — it routes through `on_navigation(ListNavigation)` from Phase 12. **Delete the Phase-12 hand-rolled `Action` / `Display` impls on `ToastsAction`** — now that the enum has a variant, declare it through the standard `action_enum!` macro so the impls are generated and consistent with every other action enum.

**Bar-renderer Toasts arm: remove the Phase-13 `Vec::new()` short-circuit.** Phase 13 ships `tui_pane/src/bar/mod.rs::pane_slots_for`'s `FocusedPane::Framework(FrameworkFocusId::Toasts)` arm as `let _ = framework.toasts.bar_slots(ctx); Vec::new()` because `ToastsAction::ALL = &[]` triggers dead-code closure inference (any `slot.primary()` body is unreachable on an uninhabited enum). Phase 24 must replace that arm with the same resolver pattern the Settings / Keymap overlay arms use — walk `framework.toasts.bar_slots(ctx)` → look up `scope.key_for(action)` against `Toasts::<Ctx>::defaults().into_scope_map()` → emit `RenderedSlot { region, label: action.bar_label(), key, state, visibility, secondary_key: None }`. With `Activate` added, the closure body becomes reachable and the borrow-check / dead-code inference issue evaporates.

**Comment cleanup.** Update stale code comments that still describe `ToastsAction::Activate` or toast lifecycle work under the old phase numbers (notably `tui_pane/src/bar/mod.rs` and `tui_pane/src/panes/toasts.rs`) so they point at Phase 24 / Phase 26 respectively.

`Toasts::handle_key` returns a command rather than mutating cross-borrow state directly:

```rust
crate::action_enum! {
    /// Actions reachable on the toast stack's local bar.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ToastsAction {
        // 3-positional because bar_label `"open"` ≠ toml_key `"activate"` (per 14.3.5).
        Activate => ("activate", "open", "Activate focused toast");
    }
}

pub enum ToastCommand<A> {
    None,
    Activate(A),
}

impl<Ctx: AppContext> Toasts<Ctx> {
    /// Pure-borrow form. Mutates pane-local state only (viewport, etc.)
    /// and returns the command for the dispatcher to apply after the
    /// framework borrow is dropped.
    pub fn handle_key_command(&mut self, bind: &KeyBind) -> (KeyOutcome, ToastCommand<Ctx::ToastAction>);
}
```

The dispatch chain calls `handle_key_command` while holding `&mut framework`, drops the borrow, then applies the command. Wire this wherever Phase 21 leaves the focused framework-pane dispatch path. If Phase 21 keeps that orchestration in `src/tui/input.rs`, wire Toasts there; if Phase 21 moves focused framework-pane dispatch into `tui_pane`, wire it there. Do not create a second focused-pane dispatch path just for Toasts.

**Focused-Toasts dispatch order.** Phase 24 changes the focused-Toasts branch left by Phase 21 so `ToastsAction` gets first claim before framework/app globals when `FocusedPane::Framework(FrameworkFocusId::Toasts)` is active. In `src/tui/input.rs::handle_key_event`, move or special-case the focused-Toasts dispatch before `dispatch_framework_global` / `dispatch_app_global`; the current Phase-22 order checks globals first and is intentionally temporary until `ToastsAction::Activate` exists. The branch resolves the key against the Toasts pane scope, calls `handle_key_command`, applies `ToastCommand::Activate` after the framework borrow is dropped, and returns `Consumed` when an action fires. If the focused toast has no action and `handle_key_command` returns `Unhandled`, dispatch then falls through to the normal globals path; this is the only intended global fallthrough from the focused-Toasts branch.

```rust
let (outcome, cmd) = ctx.framework_mut().toasts.handle_key_command(&bind);
match cmd {
    ToastCommand::None         => {},
    ToastCommand::Activate(a)  => ctx.handle_toast_action(a),
}
```

`handle_key` (the Phase 12 form) becomes a thin wrapper that calls `handle_key_command` and discards the command — kept for tests that don't care about activation. Production dispatch goes through `handle_key_command`.

**4. Cargo-port wiring.** Cargo-port adds `CargoPortToastAction`:

```rust
pub enum CargoPortToastAction {
    OpenPath(AbsolutePath),
}

impl AppContext for App {
    type AppPaneId    = AppPaneId;
    type ToastAction  = CargoPortToastAction;
    fn handle_toast_action(&mut self, action: CargoPortToastAction) {
        match action {
            CargoPortToastAction::OpenPath(path) => self.open_in_editor(&path),
        }
    }
}
```

cargo-port's existing `ToastManager::push_*` call sites (Phase 26 migration entry) accept `Option<AbsolutePath>` and convert to `CargoPortToastAction::OpenPath(path)` at the boundary. Existing call sites pass `None` (or the Phase-24 migrated `framework.toasts.push_with_action`) until the manager migration finishes.

**Phase 24 tests:**
- `enter_on_focused_toast_without_action_is_unhandled` — toast with `action: None` returns `KeyOutcome::Unhandled` for Enter; dispatch falls through to globals.
- `no_toast_action_app_compiles_with_default_handler` — a test app using `type ToastAction = NoToastAction;` and the default `handle_toast_action` body compiles.
- `handle_key_command_returns_activate_when_focused_with_action` — pure-borrow form returns the right command.

**Code touched in Phase 24:**
- `tui_pane/src/app_context.rs` — add `ToastAction` associated type, `NoToastAction` enum, default `handle_toast_action`.
- `tui_pane/src/panes/toasts.rs` — add `action` field on `Toast`, `push_with_action`, `ToastsAction::Activate`, `handle_key_command`, `ToastCommand`.
- Focused framework-pane dispatch path from Phase 21 — apply `ToastCommand` after the framework borrow ends at the actual Phase 21 focused-pane dispatch call site.
- `tui_pane/src/lib.rs` — re-export `NoToastAction`, `ToastCommand`.
- `src/tui/app/mod.rs` / `src/tui/framework_keymap.rs` (cargo-port) — define `CargoPortToastAction`, set `type ToastAction = CargoPortToastAction`, implement `handle_toast_action`.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- `AppContext::ToastAction`, `NoToastAction`, `ToastCommand`, and `Toasts::push_with_action` landed as a small additive framework API; non-activating test contexts use `NoToastAction`.
- `ToastsAction::Activate` removed the Phase-13 `Vec::new()` special case in `tui_pane/src/bar/mod.rs`; focused Toasts now render `Enter open` in the pane-action region.

**What deviated from the plan:**
- cargo-port added `impl From<AbsolutePath> for CargoPortToastAction` so `OpenPath` is constructed in production code before Phase 26 starts using it from migrated toast storage.
- The cargo-port focused-Toasts assertion covers the no-action fallthrough path by rebinding `[global] find = "Enter"` and proving Enter opens Finder when the focused framework toast has no payload.

**Implications for remaining phases:**
- Phase 26 can migrate `app.toasts` into `framework.toasts` without changing the Phase 24 activation API: payloads are already typed as `Ctx::ToastAction`, and production dispatch already applies `ToastCommand` after the framework borrow ends.
- Phase 26 owns the first production Enter-on-toast-with-action test because cargo-port visible toasts still live in `app.toasts` after Phase 24.

### Phase 24 Review

- Phase 25 was corrected from a toast-specific cargo-port persistence bridge to a framework-owned settings store. `tui_pane` owns settings path resolution, TOML load/save, SettingsPane edit state, validation display, and framework setting groups; cargo-port registers app-specific settings and side-effect callbacks.
- The old Phase 25 `ToastSettingsBinding::load` / `save` direction was rejected and removed from the plan. `ToastSettings` now loads through `SettingsStore` as the first framework-owned settings group.
- Phase 25 now includes the required temporary read-path migration for the legacy cargo-port toast manager: existing `status_flash_secs` / `task_linger_secs` reads reroute to `framework.toast_settings()` before those `TuiConfig` fields delete.
- Phase 26 now explicitly preserves persistent actionable diagnostic toasts with `push_persistent_styled(..., action: Option<Ctx::ToastAction>)`.
- Phase 26 now has a production call-site migration inventory for render, hit testing, prune/tick, viewport navigation, push/task APIs, and `DismissTarget::Toast`.

### Phase 25 — Framework settings store + SettingsPane migration (✅ landed)

Phase 25 corrects the Settings ownership boundary. Generic settings persistence is a `tui_pane` capability: file-path resolution, TOML load, TOML save, validation display, edit buffering, section rendering, and commit routing live in the framework. cargo-port supplies app-specific setting definitions and apply callbacks, but it does not own the generic SettingsPane or framework-owned setting groups.

`ToastSettings` is the first framework-owned setting group. cargo-port's existing `status_flash_secs` / `task_linger_secs` move from `TuiConfig` into the framework's `[toasts]` settings. App-specific values such as `editor`, branch names, lint flags, and CPU thresholds are registered by cargo-port as app settings; the framework persists them through the same settings store without hardcoding cargo-port concepts.

**Deliverables.** Phase 25 ships all of these together: `SettingsFileSpec`; `SettingsStore`; `SettingsSection`; registry support for app and framework sections; `ToastSettings` storage on `Framework`; SettingsPane render/edit/save support backed by the framework settings store; cargo-port registration of its app settings; migration of `status_flash_secs` / `task_linger_secs` into `[toasts]`; and deletion of the remaining binary Settings/Keymap mirror state. Do not push settings-store work downstream to the toast-manager migration — Phase 26 assumes `framework.toast_settings()` is already live, loaded, editable, and persisted by `tui_pane`.

**Settings ownership.** Phase 21 moved Settings open/input/render gating to framework overlay state, but cargo-port still owns the concrete settings row builder, popup renderer, viewport mirror, and inline error display. Phase 25 completes the intended ownership split for Settings: generic row construction, viewport/edit state, section rendering, inline validation display, text buffering, edit/commit routing, and file persistence move into `tui_pane`. cargo-port registers app setting entries and runtime side-effect callbacks only.

**Generic framework settings store.** Add a framework-owned settings store, independent of keymap TOML. Phase 27 tightened this boundary after implementation: the store is table-native and not generic over `Ctx`; `AppContext` carries no app-settings associated type. cargo-port derives `CargoPortConfig` from `store.table()` at startup, after successful settings edits, and after config reload.

```rust
pub struct SettingsFileSpec {
    pub app_id:     &'static str,
    pub file_name:  &'static str,
    pub path:       Option<PathBuf>,
}

pub struct SettingsStore {
    // owns path resolution, raw TOML document, dirty tracking,
    // validation errors, and registered setting metadata
}

pub struct LoadedSettings {
    pub store:          SettingsStore,
    pub toast_settings: ToastSettings,
}
```

`SettingsFileSpec` lets each binary identify its settings file without teaching `tui_pane` cargo-port names. `path: Some(...)` is an explicit override for tests or custom launchers; otherwise `tui_pane` resolves the config directory from `app_id` / `file_name`. The framework owns create-default, load, save, and write-error reporting.

Startup uses an explicit handoff API:

```rust
impl SettingsStore {
    pub fn load_for_startup(
        spec: SettingsFileSpec,
        registry: SettingsRegistry,
    ) -> Result<LoadedSettings, SettingsError>;
}
```

`LoadedSettings` is consumed by the binary's construction pipeline. cargo-port derives `CargoPortConfig::from_table(loaded.store.table())`, then passes `{ config, store, toast_settings }` through its startup handoff. This makes the construction sequence explicit: settings load first, app-specific config derivation second, framework/keymap setup third, `App` construction last.

**Startup app-settings target.** App settings must load before `App` exists, but the framework does not own the app's typed config struct. Phase 27 removed the `AppSettings` associated type entirely:

```rust
pub trait AppContext {
    type AppPaneId: Copy + Eq + Hash + 'static;
    type ToastAction: Clone + 'static;
    fn framework(&self) -> &Framework<Self>;
    fn framework_mut(&mut self) -> &mut Framework<Self>;
}
```

cargo-port keeps `CargoPortConfig` as its app schema / normalization type. `SettingsStore` owns the TOML table and framework settings; cargo-port derives its typed config from that table and owns runtime side effects such as lint-runtime respawn or keymap rebuild after validation succeeds.

**App setting registration.** `SettingsRegistry` is a table-native schema and row adapter, not an app-config adapter. App entries include section/key names, value kind, validation, display metadata, and codecs over `toml::Table`. App values such as cargo-port's `editor` remain app-specific fields, but the framework settings store loads and saves the TOML cells; cargo-port re-derives and applies `CargoPortConfig` after successful app-owned edits.

Framework-owned settings use the same store but do not route through cargo-port callbacks. `ToastSettings` registers a framework section named `"toasts"`; SettingsPane edits mutate `framework.toast_settings_mut()` directly and mark the settings store dirty.

**Registry ownership migration.** `SettingsRegistry` currently lives on `Keymap<Ctx>` through `KeymapBuilder::with_settings`. Phase 25 moves registry storage to `Framework<Ctx>` / `SettingsStore` and deletes the keymap-owned path. Remove `KeymapBuilder::with_settings`, the builder's `settings` field, `Keymap.settings`, and `Keymap::settings()`. Build/load `SettingsStore` before the framework keymap builder chain. Keep `register_settings_overlay()` on the keymap builder because `[settings]` shortcut bindings still belong to the keymap file; only settings values and SettingsPane rows leave keymap ownership. After Phase 25, SettingsPane never needs keymap ownership to render or edit settings.

**cargo-port config subsystem migration.** `src/config.rs` stops owning generic config path/load/save behavior. It keeps app-specific schema, defaults, validation/normalization, and migration helpers. `SettingsStore` takes over path resolution, default-file creation, TOML read/write, dirty tracking, write errors, and template generation from registered settings. Runtime reload in `src/tui/app/async_tasks/config.rs` becomes a framework settings-store reload followed by app callbacks for changed app settings; keymap reload remains on the keymap path.

**No cargo-port persistence bridge.** Do not add `ToastSettingsBinding::load`, `ToastSettingsBinding::save`, or any cargo-port-only parse/save bridge. The framework settings store owns load/save mechanics. cargo-port can keep compatibility helpers during the migration only to map old `TuiConfig` fields into the new framework settings file, and those helpers delete in this phase.

**Legacy toast-manager read path.** Phase 25 deletes `TuiConfig::status_flash_secs` and `TuiConfig::task_linger_secs`, but Phase 26 owns the `app.toasts` storage migration. Therefore Phase 25 must reroute every existing duration read used by the legacy cargo-port toast manager to `framework.toast_settings()` before deleting the config fields. Concrete reads to migrate: `App::prune_toasts`, `show_timed_toast`, `show_timed_warning_toast`, `finish_task_toast`, `set_task_tracked_items`, and the async-task helpers that currently read `self.config.current().tui.task_linger_secs`. This is a temporary read-path compatibility step only; it is not an app-owned settings copy and it deletes when Phase 26 removes `app.toasts`.

**Legacy Settings/Keymap mirror deletion.** Phase 25 also removes the remaining binary mirror state that Phase 21/22 left behind for Settings and Keymap: `Overlays::settings`, `Overlays::keymap`, the cargo-port viewport mirrors used only for those overlays, and `src/tui/input.rs::clear_legacy_framework_overlay_state`. After Phase 25, Settings/Keymap browse/edit/awaiting state lives only in the framework panes; cargo-port persists no generic SettingsPane state.

**SettingsPane edit-buffer deletion.** Phase 25 removes the app-owned Settings editor path, not just the overlay flags. Delete or migrate these concrete targets: `src/tui/config_state.rs::SettingsEditBuffer`, `src/tui/settings.rs::settings_edit_keys`, `handle_settings_edit_key`, app-side settings row render/edit helpers, any temporary settings text-input wiring in `src/tui/app/construct.rs`, and any settings inline-error mirror on `Overlays`. SettingsPane owns browse/edit/commit buffering and validation display after this phase.

**SettingsPane text-input routing.** SettingsPane no longer uses `Mode::TextInput(fn(KeyBind, &mut Ctx))` handler injection after Phase 25. It owns its edit buffer and exposes a command-returning API:

```rust
pub enum SettingsCommand<Ctx: AppContext> {
    None,
    ApplyAppSetting(SettingChange<Ctx>),
    ApplyFrameworkSetting(FrameworkSettingChange),
    Save,
    Cancel,
}

impl<Ctx: AppContext> SettingsPane {
    pub fn handle_text_input(&mut self, bind: KeyBind) -> SettingsCommand<Ctx>;
}
```

The input path borrows `framework.settings_pane`, gets a `SettingsCommand`, drops the framework borrow, then applies store mutations or app callbacks. Finder can keep the existing text-input handler path because Finder is app-owned; SettingsPane must not keep cargo-port handler injection.

**Test migration.** Phase 25 updates Phase-23-era cargo-port tests that still assert `app.overlays.is_settings_editing()` or `app.overlays.keymap_is_awaiting()` (notably in `src/tui/app/tests/framework_keymap.rs`) to assert against framework SettingsPane / KeymapPane state instead. The binary mirror deletion must not leave those tests failing by incidental field removal.

**TOML-load surface.** Phase 25's settings TOML is not a keymap overlay scope. It is loaded by `SettingsStore`, not by `KeymapBuilder::load_toml` and not by cargo-port's legacy `config.rs` parser. The shared-`[global]` action-table coordination solved in Phase 17 does not apply.

**1. `ToastSettings` with validated newtypes.** No raw `f64` durations cross the framework boundary:

```rust
pub struct ToastSettings {
    pub enabled:         bool,
    pub width:           ToastWidth,
    pub gap:             ToastGap,
    pub default_timeout: ToastDuration,
    pub task_linger:     ToastDuration,
    pub max_visible:     MaxVisibleToasts,
    pub placement:       ToastPlacement,
    pub animation:       ToastAnimationSettings,
}

pub struct ToastWidth(NonZeroU16);
pub struct ToastGap(u16);
pub struct ToastDuration(Duration);
pub struct MaxVisibleToasts(NonZeroUsize);

pub enum ToastPlacement { BottomRight, TopRight }

pub struct ToastAnimationSettings {
    pub entrance_duration: ToastDuration,
    pub exit_duration:     ToastDuration,
}
```

Construction goes through `try_from_secs(f64) -> Result<Self, ToastSettingsError>` on each newtype. `SettingsStore` converts `[toasts]` values at the settings-file boundary and reports `ToastSettingsError` with file/key context. Do not route toast-setting validation through `KeymapError`; `[toasts]` is not part of the keymap overlay loader.

**2. `SettingsRegistry` gains sections and value codecs.** The registry already supports app settings; Phase 25 adds a tagged section and enough value metadata for the framework store to load/save registered settings:

```rust
pub enum SettingsSection {
    App(&'static str),        // "tui", "lint", "cpu", etc.
    Framework(&'static str),  // "toasts", future framework groups
}

pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Enum(&'static str),
}

impl SettingsRegistry {
    pub(crate) fn add_with_section(&mut self, section: SettingsSection, entry: SettingEntry);
    // existing add_bool / add_enum / add_int become App-section helpers;
    // add_string / add_float ship here because cargo-port app settings need them.
}
```

`SettingValue` covers the common widgets; it is not the whole app-settings API. Every entry also carries codecs so cargo-port's real settings can stay in the framework SettingsPane without custom app-side editor code:

```rust
pub enum AdjustDirection { Back, Forward }

pub struct SettingCodecs {
    pub format: fn(&toml::Table) -> String,
    pub parse:  fn(&str, &mut toml::Table) -> Result<(), SettingsError>,
    pub adjust: Option<fn(AdjustDirection, &mut toml::Table) -> Result<(), SettingsError>>,
}
```

Use `parse` / `format` for strings, lists, branch names, lint commands, cache-size values, and any setting with app-specific validation. Use `adjust` for booleans, enums, steppers, and direction-aware controls. The app can provide custom codecs, but the SettingsPane still owns text input, validation display, and commit routing.

The settings pane renders sections grouped by `SettingsSection`. App sections render first in registry order; framework sections render after app sections, headed by the section name.

**3. Framework store wiring.**

`Framework<Ctx>` becomes the mutable owner of framework settings:

```rust
pub struct Framework<Ctx: AppContext> {
    pub settings_pane: SettingsPane,
    pub keymap_pane:  KeymapPane,
    pub toasts:       Toasts<Ctx>,
    settings_store:   SettingsStore,
    toast_settings:   ToastSettings,
    // ...
}
```

Phase 25 adds `toast_settings()` / `toast_settings_mut()` accessors and a framework settings initialization path. Phase 27 amends the initialization path: `SettingsStore` loads the table and framework-owned sections, while the binary derives `CargoPortConfig` from the table before `App` construction. Runtime edits use staged table mutation and validation before save/apply. The public ownership rule is fixed: `tui_pane` owns load/save mechanics, cargo-port owns only app-specific schema and side effects.

There is no `ToastSettingsBinding`:

```rust
// Do not add this:
// pub struct ToastSettingsBinding<Ctx> { load: ..., save: ... }
```

SettingsPane commits call the framework settings store's save path after mutation. If the edited entry is app-owned, the store calls the app entry's setter/apply callback, then persists. If the edited entry is framework-owned, the store mutates the framework-owned setting, then persists.

**4. cargo-port migration.** cargo-port's `TuiConfig::status_flash_secs` and `TuiConfig::task_linger_secs` move into `ToastSettings::default_timeout` / `task_linger`. The TOML schema gains a `[toasts]` section:

```toml
[toasts]
enabled         = true
default_timeout = 5.0
task_linger     = 1.0
width           = 60
gap             = 1
max_visible     = 5
placement       = "bottom_right"
```

cargo-port no longer parses or persists `[toasts]` through `src/config.rs`. `tui_pane` loads the section through `SettingsStore`, merges missing values with `ToastSettings::default()`, and saves it on SettingsPane edits. Migration precedence: explicit `[toasts]` values win; if `[toasts]` is absent, seed `default_timeout` / `task_linger` from old `[tui].status_flash_secs` / `[tui].task_linger_secs`; on save, write `[toasts]` and remove the old `[tui]` keys. The binary's settings.rs `toast_settings_rows` (`src/tui/settings.rs:286-304`) deletes for toast timing rows. `discovery_shimmer_secs` is not a toast setting; keep it registered as an app-owned cargo-port setting unless a later phase moves discovery animation into the framework.

**Phase 25 tests:**
- `settings_store_resolves_default_path_from_app_id` — framework path resolution uses `SettingsFileSpec` when no explicit path is supplied.
- `toast_settings_default_round_trip` — load a TOML with only `[toasts]` defaults through `SettingsStore`, assert framework reads the right values, save, reload.
- `legacy_tui_toast_keys_seed_toasts_section` — user config with old `[tui].status_flash_secs` / `task_linger_secs` and no `[toasts]` seeds framework `ToastSettings`; save writes `[toasts]` and drops the old keys.
- `toast_settings_invalid_width_returns_error` — `width = 0` returns `ToastSettingsError::WidthZero`.
- `app_setting_editor_round_trip` — register a fake app-owned string setting, load it from TOML, edit through SettingsPane, assert the app setter ran and the saved TOML changed.
- `config_from_table_before_app_construction` — `SettingsStore` loads the TOML table before `App` exists, and cargo-port derives `CargoPortConfig` from that table before startup side effects run.
- `legacy_toast_manager_reads_framework_settings` — before Phase 26 deletes `app.toasts`, cargo-port timed/task toast helpers read durations from `framework.toast_settings()` after the `TuiConfig` fields are gone.
- `settings_pane_renders_toast_section_after_app_section` — bar/render snapshot.
- cargo-port: `tui_config_no_longer_carries_toast_fields` — compile-fail / type-level check that the moved fields are gone.

**Code touched in Phase 25:**
- New: `tui_pane/src/settings/store.rs` — `SettingsFileSpec`, `SettingsStore`, settings TOML load/save, path resolution, dirty/error state.
- New: `tui_pane/src/toasts/settings.rs` — `ToastSettings`, validated newtypes, `ToastSettingsError`.
- `tui_pane/src/framework/mod.rs` — `settings_store: SettingsStore` and `toast_settings: ToastSettings` fields on `Framework<Ctx>`; `toast_settings()` / `toast_settings_mut()` accessors; settings initialization and save hooks.
- `tui_pane/src/settings.rs` — `SettingsSection`, `SettingValue`, app setting codecs, string/float entry support, ordering.
- `tui_pane/src/panes/settings.rs` — render section headers; route browse/edit/save through `SettingsStore`.
- `tui_pane/src/keymap/mod.rs` / `tui_pane/src/keymap/builder.rs` — remove `Keymap` ownership of `SettingsRegistry`: delete `KeymapBuilder::with_settings`, the builder `settings` field, `Keymap.settings`, and `Keymap::settings()`. Keep only keymap overlay registration for `[settings]` shortcuts.
- `tui_pane/src/lib.rs` — re-export `SettingsFileSpec`, `SettingsStore`, `SettingValue`, `ToastSettings`, the newtypes, `ToastPlacement`, `ToastAnimationSettings`, `ToastSettingsError`.
- Cargo-port: register app-specific settings with `SettingsRegistry`; migrate generic path/load/save/template behavior to `SettingsStore` while keeping app schema/normalizers in `src/config.rs`; derive `CargoPortConfig` from `SettingsStore::table()` at startup/edit/reload; update `src/tui/config_reload.rs`, `src/tui/config_state.rs`, and `src/tui/app/async_tasks/config.rs` to reload through the framework settings store; delete `status_flash_secs` / `task_linger_secs` from `TuiConfig`; migrate old values into framework `[toasts]`; reroute legacy `app.toasts` duration reads to `framework.toast_settings()`; delete toast rows from `toast_settings_rows`; delete SettingsPane edit-buffer mirrors; keep `discovery_shimmer_secs` as an app-owned setting. No `App::toast_settings` field — the framework is the sole mutable owner of framework settings.
- Phase closeout greps: `rg 'settings_edit_keys|SettingsEditBuffer|edit_buffer|overlays\\.settings|overlays\\.keymap' src/tui` must return no production-owned SettingsPane state after the migration, except deliberate compatibility comments/tests called out in the retrospective.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- `tui_pane::SettingsStore`, `SettingsFileSpec`, `SettingsRegistry`, `SettingsSection`, `SettingCodecs`, and `ToastSettings` landed as framework API, with `Framework<Ctx>` now owning `settings_store` and `toast_settings`.
- Settings/Keymap mirror state moved out of cargo-port overlays: `Overlays::settings`, `Overlays::keymap`, `SettingsEditBuffer`, `settings_edit_keys`, and the remaining `overlays.settings` / `overlays.keymap` production references are gone.
- The legacy cargo-port toast manager now reads timing through `framework.toast_settings()` while Phase 26 still owns the `app.toasts` storage migration.
- The Phase 25 closeout pass finished the missing settings ownership pieces: cargo-port registers its app settings with `SettingsRegistry`, `SettingsPane::render_rows` owns the generic row renderer / edit-buffer display / hit-target mapping, startup config load and default-file seeding run through `SettingsStore`, and runtime save/reload no longer call the old `config::save` / `config::try_load` path.

**What deviated from the plan:**
- cargo-port still owns app-specific setting labels, display values, validation helpers, and runtime side-effect orchestration in `src/tui/settings.rs`; those are intentionally app-specific. The generic row rendering, text editing, persistence, and framework-owned `[toasts]` section are framework-owned.
- `src/config.rs` remains the app schema / normalization home for `CargoPortConfig`; it no longer owns generic load/save/default-file behavior.
- Test apps initially inherited the real user config path through both `Config` and `SettingsStore`; the implementation fixed the harness by installing tempfile-backed paths in `src/tui/app/tests/mod.rs`.

**Surprises:**
- The first validation pass wrote `/tmp/test` into `/Users/natemccoy/Library/Application Support/cargo-port/config.toml` because `App::save_and_apply_config` called global `config::save(...)` instead of saving through the app instance path. The closeout fix deleted that save path; `App::save_and_apply_config` now saves through `framework.settings_store_mut().save(...)`.
- Runtime config reload needed `SettingsStore::load_from_path(...)`, not `load_current()`, because `Config::take_stamp_change()` reports the concrete changed path and tests can retarget the watcher after startup.
- Registered custom settings must translate TOML arrays into the edit-string form before calling app codecs. `tui_pane::SettingsStore` now handles string arrays and command-table arrays so normal cargo-port config files do not parse as invalid settings.
- `Viewport::set_pos` had to keep the old app viewport behavior: callers may set a cursor before the pane reports its final length, and `set_len` clamps later.

**Implications for remaining phases:**
- Phase 26 can consume `framework.toast_settings()` for toast timings without reading removed `TuiConfig` fields.
- Settings ownership is now architecturally complete for Phase 25: remaining Phase 26 work can focus on migrating `app.toasts` / `ToastManager` into `framework.toasts`, not on settings-store cleanup.
- Future tests that construct `App` must keep installing tempfile-backed `Config` and `SettingsStore` paths; otherwise test saves can mutate the real user config again.

**Verification:**
- `cargo +nightly fmt --all`
- `cargo nextest run -p cargo-port config_reload` — 9 passed
- `cargo nextest run -p cargo-port settings` — 18 passed
- `cargo nextest run -p tui_pane settings` — 25 passed
- `cargo check --workspace --all-targets -q`
- `cargo clippy --workspace --all-targets`
- `cargo nextest run --workspace` — 863 passed
- `cargo install --path .`

### Phase 25 Review

- Phase 26: setup work that Phase 25/19 already satisfied (`framework.toast_settings()` live, `handle_toast_key` absent) is now framed as acceptance checks, not implementation scope.
- Phase 26: task/tracked-item API parity now names the full cargo-port toast-manager surface that must move or be explicitly replaced before deleting `src/tui/toasts/manager.rs` / `format.rs`.
- Phase 26: task-owned operations now stay keyed by `ToastTaskId`; `ToastId` is reserved for card identity, dismissal, hitboxes, focus, and rendering.
- Phase 26: `ToastSettings` consumption now names the push/prune/render boundaries, `enabled`, `max_visible`, and the default-only animation caveat.
- Phase 26: production tests that need cargo-port `App` / `CargoPortToastAction` now stay in the binary test suite; `tui_pane/tests/` uses mock apps only.
- Phase 26: `DismissTarget::Toast` may remain as cargo-port hit-test state, but its handler must route to framework toast storage.
- Risks: removed the stale Settings toggle-direction risk after Phase 25's framework SettingsPane action model superseded it.

### Phase 26 — Migrate cargo-port `ToastManager` into `tui_pane` (✅ landed)

Phase 26 moves the generic toast subsystem from cargo-port (`src/tui/toasts/`) into the framework. Cargo-port keeps only the binary-specific copy (toast titles/bodies, which app events create toasts) and the `CargoPortToastAction` payload from Phase 24. The migration consumes `framework.toast_settings()` (added in Phase 25) for width/timing/placement — no temporary constants. The binary's old `handle_toast_key` body is already gone after Phase 19; Phase 26 verifies the symbol is still absent and focuses on deleting `app.toasts` / `ToastManager` storage and call sites.

Already-satisfied setup stays as acceptance checks, not new implementation work: `framework.toast_settings()` is live after Phase 25, and `handle_toast_key` is absent after Phase 19.

**1. Move generic types into `tui_pane/src/toasts/`.** New module structure:

```
tui_pane/src/toasts/
  mod.rs          — re-exports
  manager.rs      — ToastManager methods (was src/tui/toasts/manager.rs)
  render.rs       — toast card rendering (was src/tui/toasts/render.rs)
  format.rs       — formatting helpers (was src/tui/toasts/format.rs)
  hitbox.rs       — hit-test storage
  tracked_item.rs — TrackedItem + TrackedItemKey + TrackedItemView
```

`Toasts<Ctx>` from Phase 12 absorbs `ToastManager`'s methods directly — it is the manager. `Toast<Ctx>` (generic since Phase 12) extends to carry the lifecycle. The fields are private, so this is not a public-field break, but the storage type for the body changes from `String` to a typed `ToastBody` enum — an intentional internal representation change called out here so future implementers do not mistake it for a purely additive growth:

```rust
pub enum ToastLifetime {
    Timed   { timeout_at: Instant },
    Task    { task_id: ToastTaskId, status: TaskStatus },
    Persistent,
}

pub enum TaskStatus {
    Running,
    Finished { finished_at: Instant, linger: Duration },
}

pub enum ToastPhase {
    Visible,
    Exiting { started_at: Instant },
}

pub struct Toast<Ctx: AppContext> {
    id:             ToastId,
    title:          String,
    body:           ToastBody,
    style:          ToastStyle,
    lifetime:       ToastLifetime,
    phase:          ToastPhase,
    action:         Option<Ctx::ToastAction>,
    tracked_items:  Vec<TrackedItem>,
}

/// Typed toast body. Replaces Phase 12's `body: String` storage so the
/// renderer reads typed structure instead of treating multi-line /
/// item-list / single-line content as one undifferentiated string.
pub enum ToastBody {
    /// One-line body — the common case.
    Line(String),
    /// Multi-line body — rendered as separate rows.
    Lines(Vec<String>),
}

impl From<String> for ToastBody { /* … */ }
impl From<&str>   for ToastBody { /* … */ }
```

**`cargo mend` import locality.** Phase 16's mend pass surprised by hoisting a `#[cfg(test)] use std::path::PathBuf` to top-level (production builds then errored on the unused import). Phase 26 introduces several new public types (`ToastBody`, `ToastLifetime`, `TaskStatus`, `ToastPhase`) that thread through both binary and `tui_pane` test surfaces; `cargo mend` will rewrite their inline path-qualified uses into module-level `use` imports. After every mend pass during Phase 26, run `cargo clippy --all-targets -D warnings` against both production and test cfg, and gate any test-only imports with `#[cfg(test)]` attributes when needed.

**Boundary conversions stay stable.** Every public push entry point keeps
accepting `impl Into<String>` and converts at the boundary, so cargo-port's
existing call sites are not affected by the storage change:

- Phase 12: `Toasts::push(title: impl Into<String>, body: impl Into<String>)`,
  `Toasts::push_styled(...)`.
- Phase 24: `Toasts::push_with_action(...)`.
- Phase 26: `Toasts::push_timed(...)`, `Toasts::push_task(...)`,
  `Toasts::push_persistent(...)`, `Toasts::push_persistent_styled(...)` — all
  take `body: impl Into<String>` and convert via `ToastBody::from(s.into())`.

A second push surface for explicit multi-line bodies (`push_lines` /
`push_styled_lines`) ships in Phase 26 so call sites that already build a
`Vec<String>` do not round-trip through a single joined `String`.

**Public accessor decision: `Toast::body()` returns `&ToastBody`.** Phase 12
ships `Toast::body(&self) -> &str`; Phase 26 widens the return to
`&ToastBody`. This is the actual public-API migration. Cargo-port's
renderer is moving into `tui_pane/src/toasts/render.rs` in this same phase,
so the only out-of-tree caller is the new in-crate renderer; no binary
call sites need updating beyond the move. A `Toast::body_text()` thin
wrapper returning a flattened single-line `&str` (for tests / one-off
debug) can ship alongside if needed; if no caller wants it, drop it.

The lifetime / phase / status enums collapse cargo-port's flag set (`timeout_at` + `task_id` + `dismissed` + `finished_task` + `finished_at` + `linger_duration` + `exit_started_at` + `persistence`) into states that cannot represent invalid combinations.

**2. Move generic API onto `Toasts<Ctx>`.** Phase 12 already ships the generic skeleton: `new`, `push`, `push_styled`, `dismiss`, `dismiss_focused`, `focused_id`, `has_active`, `active`, `reset_to_first`, `reset_to_last`, `on_navigation`, `try_consume_cycle_step`, `handle_key`, `mode`, `defaults`, `bar_slots`. Phase 24 already shipped `push_with_action`, `handle_key_command`, `ToastCommand`, `ToastsAction::Activate`, focused-Toasts-before-globals dispatch, and the `Enter open` bar row. Phase 26 consumes those APIs; it does not rewire activation. The Phase 13 bar renderer reads `bar_slots`, `mode`, and `defaults` directly via `tui_pane/src/bar/mod.rs::pane_slots_for`; **Phase 26's storage move must preserve these public signatures verbatim** (`bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<ToastsAction>)>`, `mode(&self, ctx: &Ctx) -> Mode<Ctx>`, `defaults() -> Bindings<ToastsAction>`) — the bar resolver depends on them and is not migrated by Phase 26. Phase 26 adds:

```rust
impl<Ctx: AppContext> Toasts<Ctx> {
    pub fn push_timed     (&mut self, title: impl Into<String>, body: impl Into<String>, timeout: Duration) -> ToastId;
    pub fn push_task      (&mut self, title: impl Into<String>, body: impl Into<String>, linger:  Duration) -> (ToastId, ToastTaskId);
    pub fn push_persistent(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastId;
    pub fn push_persistent_styled(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        style: ToastStyle,
        action: Option<Ctx::ToastAction>,
    ) -> ToastId;
    pub fn finish_task    (&mut self, task_id: ToastTaskId);
    pub fn reactivate_task(&mut self, task_id: ToastTaskId);
    pub fn set_tracked_items(&mut self, id: ToastId, items: Vec<TrackedItem>);
    pub fn mark_item_completed(&mut self, id: ToastId, key: &TrackedItemKey);
    pub fn prune          (&mut self, now: Instant);
    pub fn render         (&self, area: Rect, buf: &mut Buffer, settings: &ToastSettings) -> Vec<ToastHitbox>;
}
```

The `push_*` entry points take raw `Duration`, not `ToastDuration` (the validated newtype from Phase 25). `ToastDuration` validates user-supplied TOML values; `push_timed` / `push_task` callers pass durations they computed in code, so the validating wrapper is unnecessary at this boundary.

`push_persistent_styled` preserves cargo-port's existing diagnostic-toast case: a persistent warning/error toast can carry both a style and an optional `CargoPortToastAction::OpenPath`. Do not collapse persistent actionable toasts into `push_persistent` without an action; keymap/config diagnostics must still open their related file after the storage migration.

**ToastSettings consumption contract.** Phase 26 makes settings ownership explicit at each boundary:
- Push helpers receive concrete `Duration`s chosen by the caller from `framework.toast_settings()`; task finish / tracked-item linger likewise receive `task_linger` from the caller or an explicit settings-aware wrapper.
- Prune/tick: `Toasts::prune(now)` stays a public method and uses `ToastSettings::default()` internally for animation timing only. Animation timing is default-only today (no TOML keys for `animation.entrance_duration` / `animation.exit_duration`), so `default()` and `framework.toast_settings()` return the same animation values; threading `&ToastSettings` through prune buys nothing. When animation TOML keys land, prune/tick will be changed to take `&ToastSettings` or be wrapped by `Framework::prune_toasts(now)` / `Framework::tick_toasts(now)`. Cargo-port-visible timing (`default_timeout`, `task_linger`) is consumed at push/finish, not at prune, so prune does not need those settings.
- Render takes `&ToastSettings`; if `enabled == false`, render emits no cards/hitboxes while storage and timers continue. `max_visible` limits the number of visible cards/hitboxes considered during rendering and focus navigation; older active toasts remain stored.
- `width`, `gap`, and `placement` are consumed by render/hitbox layout. `default_timeout` and `task_linger` are consumed when creating/finishing toasts, not retroactively on existing timers.
- `animation` is default-only in Phase 26; render and prune both read the default animation values, not user-configurable TOML.

**Task id policy.** `ToastTaskId` remains the handle for every operation tied to a running task toast. `ToastId` is the card identity for direct dismissal, hitboxes, focus, and view rendering. `push_task` may return both ids if render/hit-test code needs the card id, but cargo-port running-task state stores only `ToastTaskId`. Therefore `set_tracked_items`, `complete_missing_items`, `add_new_tracked_items`, `restart_tracked_item`, `mark_item_completed`, `tracked_item_count`, `finish_task`, `reactivate_task`, and `is_task_finished` all take `ToastTaskId`. Do not widen inflight/lint/CI/startup trackers to store both ids unless a concrete render/focus caller requires it.

**Task/tracked-item API parity.** Before deleting `src/tui/toasts/manager.rs` and `format.rs`, Phase 26 carries forward every production manager/query/helper surface currently used by cargo-port: `push_timed_styled`, `start_task`, `finish_task`, `reactivate_task`, `is_alive`, `is_task_finished`, `tracked_item_count`, `set_tracked_items`, `complete_missing_items`, `add_new_tracked_items`, `restart_tracked_item`, `mark_item_completed`, `prune_tracked_items`, `format_toast_items`, `toast_body_width`, toast hitboxes, and viewport/focused-toast cursor reads. Each either becomes a `tui_pane::Toasts<Ctx>` method / helper or is replaced by an explicitly named new API in the same phase. The closeout grep is not enough; Phase 26 must prove the running-toast call sites in `src/tui/app/async_tasks/running_toasts.rs`, `poll.rs`, `service_handlers.rs`, `repo_handlers.rs`, and `startup_phase/toast_bodies.rs` compile against the framework API without behavior loss.

**3. Cargo-port-specific types stay in cargo-port:**
- `AbsolutePath` (only used inside `CargoPortToastAction::OpenPath` — not in framework types).
- `OwnerRepo` — converts to `TrackedItemKey` via `From` at the call site.
- Concrete toast titles/bodies — passed as `String` arguments to `push_*`.
- Which app events fire `push_timed` / `push_task` / etc.

**4. Boundary conversions.** cargo-port's existing generic string conversions move with the type into `tui_pane`; the cargo-port-specific `From<AbsolutePath>` and `From<OwnerRepo>` impls stay in cargo-port in `src/tui/toast_adapters.rs`. That adapter module contains only app-domain conversions into framework types; cargo-port does not re-export framework toast APIs through a binary-side shim. Binary call sites import `tui_pane` toast types and helpers directly.

**4a. Production call-site migration inventory.** Phase 26 must name and migrate the binary call sites that currently reach `app.toasts` directly:
- `src/tui/render.rs` — render from `app.framework.toasts`, pass `app.framework.toast_settings()`, and store returned hitboxes on the framework toast manager.
- `src/tui/interaction.rs` — toast hit testing reads framework-owned hitboxes; toast body clicks focus `FrameworkFocusId::Toasts`; close clicks call `framework.toasts.dismiss(id)` followed by focus reconciliation after the framework borrow ends.
- `App::prune_toasts` and async tick paths — call the framework toast prune path with `framework.toast_settings()`, then run `reconcile_focus_after_toast_change` if the active set changed.
- All `app.toasts.push_*`, `start_task`, `finish_task`, `set_tracked_items`, and viewport navigation reads move to `app.framework.toasts`.
- `DismissTarget::Toast(id)` stays as cargo-port's hit-test / dismiss target unless the whole `DismissTarget` abstraction moves later. Its toast arm routes to `framework.toasts.dismiss(id)`; only wrappers that exist solely to reach `app.toasts` delete.

Acceptance check: `rg 'app\\.toasts|ToastManager|crate::tui::toasts|super::toasts' src/tui` must return only historical comments/tests or no production matches after Phase 26. `render_toasts` may remain only as a direct `tui_pane` import/call. `DismissTarget::Toast` may remain only as a cargo-port hit-test target whose handler routes to framework toast storage.

**5. Focus reconciliation hooks into `prune`.** Phase 12's `reconcile_focus_after_toast_change<Ctx>(ctx: &mut Ctx)` free fn runs from the framework's tick driver after `framework.prune(now)` returns — call site holds `&mut Ctx`, so the reconciler can route through `ctx.set_focus(...)` like the dispatch-time path. The dispatch-time call site (`dismiss_chain`) was wired in Phase 12 and stays untouched in Phase 26; only the new tick-driver call site is added here. Toast mutations that can drop the active count to zero (`dismiss`, `dismiss_focused`, `prune`, `finish_task` when linger is zero) only mutate the toast vec; they never touch focus directly. Focus repair is always a separate post-mutation step at a `&mut Ctx`-holding call site.

**6. Mode integration.** Phase 12 already returns `Mode::Navigable` for focused Toasts; Phase 26 has no work here.

**7. Verify the binary's `handle_toast_key` function body is already gone.** Phase 19 deleted the legacy call path and function body. Phase 26 does not re-delete that symbol; it verifies `rg 'handle_toast_key' src/tui/input.rs` stays empty while the remaining toast ownership cleanup deletes `app.toasts`, the cargo-port `ToastManager`, and the old push/prune/render call sites.

**Phase 26 tests:**
- cargo-port production tests stay in the binary test suite and use the tempfile-backed `make_app` harness: `enter_on_focused_toast_with_action_dispatches` — after `app.toasts` has migrated into `framework.toasts`, fixture toast with `CargoPortToastAction::OpenPath(p)` set; Enter on the focused toast calls `handle_toast_action(OpenPath(p))`.
- cargo-port production: `persistent_diagnostic_toast_keeps_action_path` — migrated keymap/config diagnostic toast uses `push_persistent_styled(..., Some(CargoPortToastAction::OpenPath(path)))`, and Enter opens that path.
- `tui_pane` lifecycle tests: `timed_toast_expires_at_timeout_at`, `task_toast_lingers_after_finish_then_prunes`, `persistent_toast_survives_prune`.
- `tui_pane` tracked-item tests: `set_tracked_items_then_mark_completed_renders_strikethrough`, `prune_tracked_items_removes_finished_after_linger`.
- `tui_pane` hitbox test: `render_emits_card_and_close_hitbox_per_visible_toast`.
- `tui_pane` focus reconciliation test: `prune_emptying_active_set_while_focused_moves_focus_to_first_live_app_tab_stop`.
- Cross-crate framework test: cargo-port's `App::push_timed_toast` behavior moves to a `tui_pane/tests/` integration test that uses a `MockApp` with `type ToastAction = NoToastAction;` (test pushes `action: None` only — `NoToastAction` is uninhabited, so any `Some(action)` constructor is statically impossible).
- `tui_pane` settings/render test: `render_uses_framework_toast_settings_width` — render output reflects a non-default `ToastWidth` set on `Framework::toast_settings`.

**Code touched in Phase 26:**
- New: `tui_pane/src/toasts/{mod,manager,render,format,hitbox,tracked_item}.rs`.
- `tui_pane/src/panes/toasts.rs` — `Toasts<Ctx>` becomes a thin re-export of `tui_pane::toasts::Toasts<Ctx>`, or merged into it.
- `tui_pane/src/lib.rs` — re-export `Toast`, `ToastLifetime`, `ToastPhase`, `TaskStatus`, `ToastTaskId`, `ToastStyle`, `TrackedItem`, `TrackedItemKey`, `TrackedItemView`, `ToastView`, `ToastHitbox`.
- Cargo-port: delete `src/tui/toasts/{manager,render,format}.rs` and the binary-side `src/tui/toasts/mod.rs` shim. The cargo-port-specific `From` impls for `TrackedItemKey` live in `src/tui/toast_adapters.rs`; every other toast type/helper is imported directly from `tui_pane`.
- Cargo-port `App` shrinks: `app.toasts: tui_pane::Toasts<App>` is `app.framework.toasts` directly; the field on `App` deletes. All `app.toasts.push_*` call sites become `app.framework.toasts.push_*`.
- `App::dismiss(DismissTarget::Toast(id))` stays only if `App::dismiss` still owns non-toast dismissal; its toast arm routes to `framework.toasts.dismiss(id)`. Toast-only helpers that only wrapped `app.toasts.dismiss(id)` delete.
- `src/tui/input.rs` — verify `handle_toast_key` remains absent; no Phase 26 code deletion is expected for that symbol.
- Phase closeout runs the remaining-phase closeout gate.

### Retrospective

**What worked:**
- `Toasts<Ctx>` absorbed cargo-port's old manager surface directly; cargo-port now pushes, renders, hit-tests, prunes, and dispatches toast actions through `app.framework.toasts`.
- `ToastTaskId` stayed the handle for task-owned operations, while `ToastId` became the card identity for hitboxes, focus, direct dismissal, and rendering.

**What deviated from the plan:**
- The framework module landed as `tui_pane/src/toasts/{mod,manager,render,format}.rs`; `hitbox.rs` and `tracked_item.rs` stayed folded into `manager.rs` instead of separate files.
- The temporary binary-side toast shim was removed in Phase 26 closeout. Cargo-port-specific `TrackedItemKey` conversions now live in `src/tui/toast_adapters.rs`; framework toast APIs are imported directly from `tui_pane`.
- `ToastSettings.animation` now drives line reveal/collapse timing from the existing default values; Phase 26 did not add user-editable TOML keys for animation timing.

**Surprises:**
- Focus reconciliation depends on live toasts, not renderable toasts. `Toasts::has_active()` and `Toasts::is_alive()` now exclude exiting cards, while `active_views()` keeps exiting cards renderable for animation.
- `tui_pane::Viewport` and cargo-port's app-pane `Viewport` are distinct types, so focused-toast hit testing stays a direct framework-toast path instead of sharing the binary's app-pane viewport helper.

**Implications for remaining phases:**
- Phase 27 is a required settings-boundary correction, not final closeout. It removes the last app-config generic from the framework settings store.
- The remaining migration cleanup is required and stays numbered: Phase 28 finishes framework pane/API cleanup, Phase 29 finishes toast API cleanup, Phase 30 moves generic overflow affordance ownership into the framework, and Phase 31 is final closeout.
- Final closeout must grep for stale live-code references to `app.toasts`, `ToastManager`, `handle_toast_key`, cargo-port-only toast constants, and legacy keymap/settings UI ownership; historical phase notes can keep those terms.

### Phase 26 Review

- Replaced the earlier final-closeout assumption with required remaining phases. Phase 27 owns the settings-boundary correction; final closeout moves to Phase 31.
- Updated the Phase 26 `ToastSettings` prune/tick contract: `Toasts::prune(now)` stays public and uses default-only animation timing; user-configurable toast timings are consumed at push/finish, not prune.
- Reconciled the `Definition of done` public export list with shipped `tui_pane` exports, including the Phase 26 toast surface and the Phase 26 closeout status-line surface.
- Removed the binary-side toast shim in code and plan text. cargo-port keeps only `src/tui/toast_adapters.rs` for app-domain `TrackedItemKey` conversions; all toast APIs are imported directly from `tui_pane`.
- Moved final status-line ownership into `tui_pane::render_status_line(...)`: the framework now owns fill, uptime / scanning text, left/center/right placement, global-strip composition, per-slot resolution, and styling. cargo-port supplies only app facts, global-slot policy, and palette data.
- Rewrote stale test-infrastructure and risk prose from future-plan language into shipped-code inventory.

### Phase 27 — Resolve settings boundary (✅ landed)

Phase 27 removes `CargoPortConfig` from the framework settings type graph. `SettingsStore`, `SettingsRegistry`, `SettingCodecs`, `LoadedSettings`, and `ReloadedSettings` are table-native and no longer generic over `Ctx`; `AppContext` has no `AppSettings` associated type.

Boundary details:
- Before this phase, `SettingsStore<Ctx>`, `SettingsRegistry<Ctx>`, `LoadedSettings<Ctx>`, and `ReloadedSettings<Ctx>` flowed `Ctx::AppSettings` through the framework. cargo-port set that associated type to `CargoPortConfig`, which made the framework depend on an app-specific config struct.
- Settings entries now read and mutate `toml::Table`, not `CargoPortConfig`. `SettingKind`, `SettingEntry`, `SettingCodecs`, and every `SettingsRegistry::add_*` method are table-native.
- `SettingsStore::save(&mut self)` writes the in-memory table directly. It does not take app settings or rebuild a table from typed app config.
- The framework owns file path resolution, TOML load/save, dirty state, validation display, and framework setting groups. cargo-port owns only app-specific schema, normalization, and runtime side effects.

Deliverables:
- `SettingsStore` owns the TOML table and saves it directly. Runtime edits use staged table mutation, validation, rollback on error, and then `SettingsStore::save()`.
- cargo-port derives `CargoPortConfig` from `SettingsStore::table()` at startup, after successful app-owned settings edits, and after config reload. The framework never stores or mutates `CargoPortConfig`.
- Framework-owned settings are derived from the same table and cached inside `Framework`.
- `settings_table_from_config` is limited to startup/default-file seeding and test setup. Runtime save paths must not rebuild a fresh table from typed app config.
- Validation: format, check, clippy, workspace nextest, and install.

### Phase 28 — Framework pane/API cleanup (✅ landed)

Finish the remaining framework pane/API boundary work after the settings store is table-native.

Deliverables:
- Move key-capture text-input state and mutation out of cargo-port and into `tui_pane::KeymapPane`.
- Replace stringly `SettingsRow.payload` with a typed payload/newtype that identifies the registered setting row without encoding app-specific structure in the renderer.
- Remove unnecessary `<Ctx>` from `SettingsPane`, settings view, render, and helper types that no longer need app context after the table-native settings boundary.
- Keep app-specific schema, labels, validation, and side effects in cargo-port registry callbacks; keep generic row building, edit state, validation display, and persistence mechanics in `tui_pane`.
- Define one reusable construction sequence for settings load, settings store installation, keymap TOML load, framework pane setup, and app construction handoff. A second client should not need to copy cargo-port builder internals to use settings + keymap loading.
- Delete binary-side handler-injection, capture, or construction shims made obsolete by the framework-owned paths.
- Acceptance tests cover key rebind capture, cancel/escape, invalid duplicate handling, save/reload, text-input suppression, row identity stability, edit/rollback behavior, restart persistence, and a small second-client-style settings + keymap load fixture.

### Retrospective

**What worked:**
- `KeymapPane` now owns capture state transitions and exposes `handle_capture_key(...) -> KeymapCaptureCommand`. cargo-port still validates conflicts and persists TOML because those are app/keymap-file concerns, but the generic capture state is framework-owned.
- `SettingsPane` and `KeymapPane` no longer carry stored `<Ctx>` type parameters. Their `mode` methods stay generic only at the call boundary so they can return `Mode<Ctx>`.
- `SettingsRow.payload` is now a typed `SettingsRowPayload` newtype and `SettingsPane` stores typed line targets internally.
- `Framework::new_with_settings(...)` / `install_loaded_settings(...)` give clients a reusable startup handoff for loaded settings and framework pane setup; cargo-port no longer performs separate settings-store/toast-setting installation steps.
- Stale plan text that pointed at binary-side text-input handler storage was rewritten to the current command-returning pane API.

**What deviated from the plan:**
- Keymap capture still calls back into cargo-port for app-specific validation and persistence. That is the right split: `tui_pane` owns capture mode/cancel/conflict state; cargo-port owns conflict policy against its scopes and the disk write/reload side effects.
- Keymap `Conflict` mode remains `Mode::Static`, not `Mode::TextInput`, so the conflict bar actions stay visible. `src/tui/input.rs::focused_text_input_mode` explicitly treats Keymap capture/conflict as a suppression boundary so structural/global shortcuts still do not leak through.
- `SettingsRowPayload` is intentionally a typed row identity wrapper, not a cargo-port `SettingOption` enum. cargo-port keeps its app-specific schema and row mapping.

**Verification:**
- `cargo +nightly fmt`
- `cargo check --workspace --all-targets`
- `cargo nextest run -p tui_pane keymap`
- `cargo nextest run -p tui_pane settings`
- `cargo nextest run -p cargo-port keymap`
- `cargo nextest run -p cargo-port settings`
- `cargo clippy --workspace --all-targets`
- `cargo nextest run --workspace` — 845 passed
- `cargo install --path .`

### Phase 29 — Toast API boundary cleanup (✅ landed)

Finish the toast API boundary now that toast storage/rendering lives in `tui_pane`.

Deliverables:
- Stabilize the public toast surface, including `ToastSettings`, task lifecycle, hitbox/focus, render helpers, and tracked-item types.
- Replace cargo-port-specific `From<AbsolutePath> for TrackedItemKey` style adapters with explicit app-side conversion helpers. The framework toast crate must not know cargo-port path/domain types.
- Audit surviving `<Ctx>` on toast-facing types. Keep it only where the type genuinely carries `Ctx::ToastAction`; remove it from view/helper types that do not need app context.
- Acceptance tests cover task finish/linger timing, tracked-item overflow, focused toast action dispatch, and app-domain conversion helpers.

End-of-phase spinner cleanup:
- Move the generic activity-spinner primitive out of cargo-port and into `tui_pane`. Cargo-port may decide that a lint run is `Running`, but the frame set, timing, and `frame_at(elapsed)` logic are framework UI chrome.
- Replace cargo-port's `src/tui/animation.rs::LINT_SPINNER` call sites with the framework-owned spinner export. This includes the Lints table running row and the Package pane's lint-status row so both match the rest of the app's running indicators.
- Framework toast rendering uses the same spinner primitive instead of a private toast-only `SPINNER_FRAMES` array.
- Acceptance tests pin that the Lints pane, Package lint row, and toast tracked-item row all render frames from the same framework spinner cycle at the same elapsed timestamps.

### Retrospective

**What worked:**
- `TrackedItemKey` is now the framework's tracked-item identity type throughout toast storage; cargo-port domain conversions live behind explicit `toast_adapters::{path_key, owner_repo_key}` helpers.
- The activity spinner moved into `tui_pane`; Lints, Package lint rows, and toast tracked-item rows now share the same `ACTIVITY_SPINNER` cycle.
- Toast view/render helper types stayed non-`Ctx`; `Ctx` remains only on storage/dispatch types that carry `Ctx::ToastAction`.

**What deviated from the plan:**
- `mark_item_completed` now takes `&TrackedItemKey`; the string-key public wrapper remains as `mark_tracked_item_completed` for compatibility at call sites that only have raw string keys.
- Generic `From<String>` / `From<&str>` for `TrackedItemKey` remain because they are framework-owned primitive conversions, not cargo-port domain adapters.

**Surprises:**
- Reusing the braille activity spinner in toast rows required display-width accounting in `tracked_item_line`; byte length would over-pad and misalign Unicode frames.
- `Toasts<Ctx>` no longer needs a private `PhantomData` field because it already stores `Vec<Toast<Ctx>>`.

**Implications for remaining phases:**
- Phase 30 owns only the remaining generic overflow-affordance move; spinner ownership is complete.
- Final closeout should include stale-reference greps for `src/tui/animation.rs`, `LINT_SPINNER`, and toast-only `SPINNER_FRAMES` in addition to the existing toast/keymap/settings cleanup checks.

### Phase 29 Review

- Phase 30 now excludes Toasts from the generic viewport overflow-affordance migration. Toast tracked-item overflow remains the existing in-card `(+N more)` row.
- Phase 30 now splits already-participating app-pane migration from overlay onboarding. Finder must expose the needed viewport state; Settings and Keymap either render through a constrained scroll body with viewport state or stay explicitly excluded until they do.
- Phase 30 now names `tui_pane::Viewport` as the owner of overflow label calculation so cargo-port's app-pane viewport does not remain a second source of truth.
- Phase 31 closeout now greps for stale Phase 29/30 paths: `src/tui/animation.rs`, `LINT_SPINNER`, toast-only `SPINNER_FRAMES`, `pane::render_overflow_affordance`, `src/tui/pane/state.rs::overflow_affordance`, and live-code overflow literals outside the framework-owned path.

### Phase 30 — Framework viewport overflow affordance

Move the generic `more ▼` / `▲ more ▼` / `▲ more` scroll affordance out of cargo-port pane chrome and into the framework viewport/chrome layer before final closeout.

Deliverables:
- Framework owns overflow-affordance calculation and rendering for scrollable pane bodies and framework-style overlay bodies. The binary may supply app rows/content, but it does not hand-render generic "more" indicators.
- Move the overflow label calculation onto `tui_pane::Viewport`. cargo-port's app-pane viewport may temporarily forward to that framework rule during migration, but `src/tui/pane/state.rs::Viewport::overflow_affordance` must not remain a second source of truth.
- Toasts are excluded from this migration. Toast tracked-item overflow stays represented by the existing in-card `(+N more)` row; toast stack visibility remains governed by `ToastSettings::max_visible` and focused-toast navigation, not the generic `more ▼` / `▲ more` pane affordance.
- Finder's popup results path participates in the same overflow affordance path as other scrollable panes. Opening Finder with more results than visible rows shows `more ▼`; scrolling down updates to `▲ more ▼`; reaching the end shows `▲ more`.
- First migrate the already-participating app-pane callers from `pane::render_overflow_affordance` to the framework-owned helper: Project List, Package, Git, Targets, Lang, Lints, and CI.
- Then onboard overlay/body targets one by one after each exposes the required viewport state. Finder must set visible row count and scroll offset for its result body. Settings and Keymap must either render through a constrained scroll body with viewport state or be explicitly excluded until they do. Each target gets an acceptance test for top/middle/bottom overflow labels before it is marked migrated.
- The framework-owned helper uses `Viewport::len`, `scroll_offset`, and visible row count as its source of truth; individual panes must set those values, not duplicate overflow math.
- Acceptance tests render at least one normal pane and Finder in constrained heights and assert the correct affordance text appears and changes with scroll position.

### Phase 31 — Final closeout

Phase 31 is the final cleanup checkpoint after Phases 28-30. It does not add new architecture.

Deliverables:
- Reconcile `What dissolves`, `What survives`, `Risks and unknowns`, `Definition of done`, and `Non-goals` against shipped code.
- Run stale-reference greps for deleted live-code paths: `app.toasts`, `ToastManager`, `handle_toast_key`, old cargo-port toast constants, `src/tui/animation.rs`, `LINT_SPINNER`, toast-only `SPINNER_FRAMES`, `pane::render_overflow_affordance`, `src/tui/pane/state.rs::overflow_affordance`, live-code `more ▼` / `▲ more` literals outside the framework-owned affordance path, legacy bar/keymap/focus names, legacy settings/keymap overlay ownership, and cargo-port-only framework shims.
- Remove or justify any remaining temporary lint `expect` attributes in the framework crate.
- Run the final validation stack and install the binary.

---

## What dissolves

- Every `KeyCode::*` direct match used for configurable command dispatch. Structural preflights, modal confirmation, and text-input editing fallback are explicit survivors.
- `App::enter_action`; `shortcuts::enter()` const fn.
- The seven hardcoded `Shortcut::fixed(...)` constants.
- The four per-context group helpers in `shortcuts.rs`.
- The threaded gating parameters in `for_status_bar`.
- `NAVIGATION_RESERVED` / `is_navigation_reserved`.
- `is_vim_reserved`'s hardcoded `VIM_RESERVED` table (replaced by reading `NavigationAction`'s bindings).
- The `+`/`=` parser collapse.
- `is_legacy_removed_action` — removed in Phase 23 after cargo-port began moving legacy `[project_list] open_editor` / `rescan` keys to `[global]` before validation.
- `InputContext` enum.
- cargo-port-owned toast storage and rendering (`app.toasts`, `ToastManager`, `src/tui/toasts/*`) — moved into `tui_pane::Toasts<Ctx>` / `tui_pane::render_toasts`; cargo-port keeps only `src/tui/toast_adapters.rs` for app-domain conversions.
- cargo-port-owned status-line construction — `tui_pane::render_status_line` owns full-line fill, uptime / scanning text, global strip composition, placement, and styling. cargo-port supplies app facts, global-slot policy, and palette data.

## What survives

- `Pane` trait — remains the app-pane contract. It gained `tab_stop()` during Phase 20.1 so the framework owns focus traversal; the bar refactor itself did not add pane-rendering requirements.
- Per-pane host structs — keep their rendering/content ownership and gain framework trait impls (`Shortcuts`, tab metadata, dispatch hooks) without moving app-specific pane body rendering into `tui_pane`.
- `GlobalAction::Dismiss` — keeps `'x'` as the single dismiss action. Routed through `dismiss_chain` (Phase 12 free fn) which calls `framework.dismiss_framework()` first (focused-toast dismiss owned by `tui_pane`, then `close_overlay`), then the binary's optional `dismiss_fallback` hook. There is no separate `ToastsAction::Dismiss`; binaries that want Esc to dismiss focused toasts rebind `GlobalAction::Dismiss`.
- Vim-mode opt-in semantics — `h`/`j`/`k`/`l` still gated by `VimMode::Enabled`.
- **Public bar surface:** `tui_pane::StatusBar`, `tui_pane::StatusLine`, `tui_pane::StatusLineGlobal`, `tui_pane::BarPalette`, `tui_pane::render_status_bar(focused, ctx, keymap, framework, &BarPalette) -> StatusBar`, `tui_pane::render_status_line(...)`, plus the accessors `Keymap::render_navigation_slots`, `Keymap::render_app_globals_slots`, and `Keymap::render_framework_globals_slots`. The framework owns region partitioning, suppression rules, per-slot resolution, global-strip composition, full-line fill, uptime / scanning text, left/center/right placement, and the styling pass. The binary supplies app facts and theme data only.
- `src/keymap.rs` remains as cargo-port's compatibility/template/migration layer for the keymap file path and the legacy TOML schema. Runtime configurable dispatch goes through `tui_pane::Keymap`; the legacy compatibility layer is not a shortcut-dispatch owner.

---

## CI tooling sanity check

Verified during the migration. Closeout validation runs workspace-scoped Cargo commands (`cargo check --workspace --all-targets`, `cargo mend --workspace --all-targets`, `cargo clippy --workspace --all-targets`, `cargo nextest run --workspace`) plus `cargo install --path .` for the binary install path.

## Doctest + test infrastructure

- No doctests. Code blocks in `///` comments are ` ```ignore ` or prose.
- **Shipped pattern (Phases 1–11):** unit tests live next to their module (`#[cfg(test)] mod tests`); each test module declares its own inline `TestApp` struct with an `AppContext` impl rather than going through a shared `test_support/` module. The duplication is small (~10 sites) and keeps each module's tests self-contained. New phases continue this pattern unless test fixtures grow large enough to consolidate.
- **Cross-crate macro test:** the framework crate's `tests/macro_use.rs` exercises `tui_pane::action_enum!` and `tui_pane::bindings!` from outside the crate. Phases 5/6/7 extended this; Phase 12+ continues to extend it whenever a new `#[macro_export]` macro lands (standing rule 6).
- **Cross-module integration tests** under the framework crate's `tests/` directory cover the shipped public framework surface in `framework_bar.rs`, `framework_toasts.rs`, and `macro_use.rs`. Integration tests declare their own context types inline and verify the public API without privileged access to crate internals (`#[cfg(test)]` modules of an upstream crate are unreachable from `dev-dependencies` — the boundary is enforced by the language, not convention).
- **Toast tests after Phase 26:** lifecycle / tracked-item / focused-command behavior is unit-tested in the framework crate's toast manager; framework-owned toast rendering is covered by `tests/framework_toasts.rs`; cargo-port production action dispatch is covered in `src/tui/app/tests/framework_keymap.rs`.
- **Binary-side test support** (`src/tui/test_support.rs`, `pub(super) fn make_app`) stays separate from the framework's tests. Phase 16 hoisted `make_app` from `tests/mod.rs` into that module. Dependency direction is binary → library only, so the framework crate's tests cannot reach binary fixtures, and binary tests cannot reach the framework crate's `cfg(test)` modules.
- **No third `*-test-support` crate.** The two fixture sets are disjoint by language rule.

## Risks and unknowns

- **Workspace conversion.** Verified during Phase 1; no further action. Both crates build green, `cargo install --path .` still installs the binary, `Cargo.lock` and `target/` are unchanged in location.
- **Framework public API completion.** cargo-port has now consumed the framework through the keymap, bar, overlay, settings, focus, and toast paths, but the migration is not complete until Phases 28-30 finish the remaining required boundary work: keymap capture ownership, settings row/view cleanup, reusable construction lifecycle, toast API/adapters, and generic viewport overflow affordance ownership.
- **Scope precedence.** `NavigationAction::Right` and `ProjectListAction::ExpandRow` both default to `Right`. The "pane scope wins" rule is documented above and enforced by the input router. Lock with a unit test.
- **`is_vim_reserved` load order.** It must read `Navigation::defaults()` (constant builder), not the in-progress keymap, to avoid a load-order cycle when called inside `resolve_scope`. Defaults are constant and always available.
- **Framework grants `&mut Vec<Span>` to bar code.** Framework convention: each helper pushes only into vecs it owns content for. Reviewed at PR time.
- **Existing user TOML configs.** New scope names (`[finder]`, `[output]`, `[navigation]`, …) are additive; old configs without these tables still parse and use defaults. No breaking change.

---

## Definition of done

- Workspace exists with `tui_pane` member crate; binary crate consumes it.
- `tui_pane` exposes every supported public type at the crate root — `tui_pane::Foo` flat, never `tui_pane::keymap::Foo`: `AppContext`, `NoToastAction`, `BarPalette`, `BarRegion`, `BarSlot<A>`, `ShortcutState`, `StatusBar`, `StatusLine`, `StatusLineGlobal`, `StatusLineGlobalAction`, `Visibility`, `render_status_bar`, `render_status_line`, `status_line_global_spans`, `CycleDirection`, `Framework<Ctx>`, `ListNavigation`, `TabOrder`, `TabStop`, `Action`, `Bindings<A>`, `bindings!`, `Configuring`, `GlobalAction`, `Globals<Ctx>`, `KeyBind`, `KeyInput`, `KeyOutcome`, `KeyParseError`, `Keymap<Ctx>`, `KeymapBuilder<Ctx>`, `KeymapError`, `Navigation<Ctx>`, `Registering`, `RenderedSlot`, `ScopeMap<A>`, `Shortcuts<Ctx>`, `VimMode`, `Mode<Ctx>`, `Pane<Ctx>`, `FocusedPane`, `FrameworkFocusId`, `FrameworkOverlayId`, `KeymapPane`, `KeymapPaneAction`, `SettingsCommand`, `SettingsPane`, `SettingsPaneAction`, `SettingsRender`, `SettingsRenderOptions`, `ToastsAction`, `AdjustDirection`, `LoadedSettings`, `MaxVisibleToasts`, `ReloadedSettings`, `SettingAdjuster`, `SettingCodecs`, `SettingEntry`, `SettingKind`, `SettingValue`, `SettingsError`, `SettingsFileSpec`, `SettingsRegistry`, `SettingsRow`, `SettingsRowKind`, `SettingsSection`, `SettingsStore`, `ToastAnimationSettings`, `ToastDuration`, `ToastGap`, `ToastPlacement`, `ToastSettings`, `ToastWidth`, `Toast`, `ToastBody`, `ToastCommand`, `ToastHitbox`, `ToastId`, `ToastLifetime`, `ToastPhase`, `ToastRenderResult`, `ToastStyle`, `ToastTaskId`, `ToastTaskStatus`, `ToastView`, `Toasts<Ctx>`, `TrackedItem`, `TrackedItemKey`, `TrackedItemView`, `format_toast_items`, `render_toasts`, `toast_body_width`, `Viewport`, and `action_enum!`. The `__bindings_arms!` helper macro is `#[doc(hidden)]` but technically reachable as `tui_pane::__bindings_arms!` (a side-effect of `#[macro_export]`); it is not part of the supported surface.
- `ScopeMap::by_action: HashMap<A, Vec<KeyBind>>`; `display_keys_for(action) -> &[KeyBind]` exists; primary-key invariant locked.
- TOML parser accepts `key = "Enter"` and `key = ["Enter", "Return"]`; rejects in-array duplicates and cross-action collisions within a scope.
- `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction` exist in cargo-port. `ProjectListAction` has `ExpandRow` / `CollapseRow`.
- Every cargo-port app pane has `impl Shortcuts<App>`. `AppNavigation: Navigation<App>`. `AppGlobalAction: Globals<App>`.
- `App.framework: Framework<App>` field exists.
- Every configurable command shortcut dispatches through the keymap; no `KeyCode::*` direct match for command keys remains. Structural preflights, modal confirmation, and text-input editing fallback may still inspect concrete keys.
- `NAVIGATION_RESERVED`, `is_navigation_reserved`, hardcoded `VIM_RESERVED`, the seven `Shortcut::fixed` constants, the four group helpers, `App::enter_action`, `shortcuts::enter()`, `InputContext` enum — all deleted.
- Framework owns the bar: region partitioning, global-strip composition, full-line fill, uptime / scanning text, left/center/right placement, per-slot resolution, and styling. cargo-port supplies app facts, global-slot policy, and palette data, but does not construct shortcut spans or lay out the status line.
- `make_app` hoisted to `src/tui/test_support.rs`.
- Bar output for every focused-pane context is snapshot-locked under default bindings against the new static-label framework bar (`render_status_bar` + `cargo_port_bar_palette()`). The snapshots are not byte-identical to the pre-refactor bar — Phase 14's `bar_label` collapse drops today's row-dependent labels in favor of one static literal per variant.
- All final validation passes: format, check, mend, clippy, workspace nextest, and install.

---

## Non-goals

- Not changing pane body render code. The `Pane` trait already gained `tab_stop()` in Phase 20.1 for framework-owned traversal; Phase 27 does not add another pane API change.
- Not unifying `PaneId::is_overlay()` semantics across the codebase — `InputContext` is being deleted, so the asymmetry resolves itself.
- Not making typed-character text input (Finder query, Settings numeric edit) keymap-driven — that's not what the keymap is for.
- Not extracting `FinderPane` into the framework crate in this migration. Finder remains app-owned.
- Not rewriting user TOML beyond the compatibility migrations already needed for the framework keymap path. Old configs parse cleanly via additive tables plus the Phase 23 legacy-key migration.
