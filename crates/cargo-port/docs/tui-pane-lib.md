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
| `paneid-ctx.md` | `tui_pane::PaneId` (framework panes) / `AppPaneId` (binary) / `FocusedPane` (wrapping), `AppContext` trait, focus tracking, `App` field changes, migration order, `InputMode` query plumbing |
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
        │   ├── action_enum.rs     # ActionEnum trait + action_enum! macro
        │   ├── global_action.rs   # GlobalAction enum + ActionEnum impl
        │   │                       #   + GlobalAction::defaults() (added Phase 4)
        │   ├── shortcuts.rs        # Shortcuts<Ctx> trait (Phase 7)
        │   ├── navigation.rs       # Navigation<Ctx> trait (Phase 7)
        │   ├── globals.rs          # Globals<Ctx> trait (Phase 7)
        │   ├── builder.rs          # KeymapBuilder<Ctx>; register, with_navigation,
        │   │                       #   with_globals, with_settings, vim_mode,
        │   │                       #   builder(quit, restart, dismiss)
        │   ├── vim.rs              # VimMode enum (Phase 3); vim-binding
        │   │                       #   application + vim_mode_conflicts +
        │   │                       #   is_vim_reserved fns added in Phase 9
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
        │                           #   input_mode_queries registry;
        │                           #   editor_target_path,
        │                           #   focused_pane_input_mode
        └── panes/                  # framework-internal panes
            ├── mod.rs
            ├── keymap_pane.rs      # KeymapPane<Ctx>;
            │                       #   Mode::{Browse, Awaiting, Conflict}
            ├── settings_pane.rs    # SettingsPane<Ctx>;
            │                       #   Mode::{Browse, Editing}
            └── toasts.rs           # Toasts<Ctx>; ToastsAction::Dismiss
```

App-specific code stays in the binary crate. Framework code lives only in `tui_pane/src/`.

**Conceptual module dependencies** (Rust modules within one crate compile as a unit, so this is a readability layering, not a hard ordering):
- `keymap/` — bindings storage + traits + builder. The builder is the keymap's builder; it calls into `framework.rs` and `settings.rs` to file pane queries and settings during registration, but the resulting `Keymap<Ctx>` is the build product.
- `bar/` — reads `Keymap<Ctx>` and pane `Shortcuts<Ctx>` impls; emits `StatusBar`.
- `panes/` — framework panes implementing `Shortcuts<Ctx>`.
- `settings.rs` — `SettingsRegistry<Ctx>`.
- `framework.rs` — aggregates framework panes and the `input_mode` query registry.
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

Linking the runtime tag to the compile-time pane type: every `Shortcuts<App>` impl declares `const APP_PANE_ID: AppPaneId`. Calling `register::<PackagePane>()` records that value alongside the pane's dispatcher — registration populates the runtime mapping. The `AppPaneId` enum is the runtime side of the same registration.

Cargo-port's existing `tui::panes::PaneId` enum becomes a type alias `pub type PaneId = tui_pane::FocusedPane<AppPaneId>;` so existing call sites that name `PaneId` keep compiling; only the framework variants move out of the enum body.

See `paneid-ctx.md` §1 for full type definitions and call-site rewrites.

### `Shortcuts` — pane scopes (state-bearing)

```rust
pub trait Shortcuts<Ctx: AppContext>: 'static {
    type Action: ActionEnum + 'static;
    const SCOPE_NAME: &'static str;

    fn defaults() -> Bindings<Self::Action>;

    /// Per-frame bar label for `action`. `None` hides the slot.
    /// Default returns `Some(action.bar_label())` (the static label
    /// declared in `action_enum!`). Override only when the label
    /// depends on pane state — e.g. `PackageAction::Activate` reads
    /// `"open"` on `CratesIo` fields and `"activate"` elsewhere.
    fn label(&self, action: Self::Action, _ctx: &Ctx) -> Option<&'static str> {
        Some(action.bar_label())
    }

    /// Per-frame enabled / disabled status for `action`. Default
    /// `Enabled`. Override when the action is visible but inert (e.g.
    /// `PackageAction::Clean` grayed out when no target dir exists).
    fn state(&self, _action: Self::Action, _ctx: &Ctx) -> ShortcutState {
        ShortcutState::Enabled
    }

    /// Bar slot layout. Owned `Vec`; cheap (N ≤ 10) and ratatui's
    /// per-frame work dwarfs the allocation. Each slot carries the
    /// `BarRegion` it lands in; most panes return
    /// `(BarRegion::PaneAction, Single(action))` for every action, but
    /// ProjectList additionally returns `(BarRegion::Nav, Paired(…))`
    /// for its expand/collapse pairs. Default impl returns one
    /// `(PaneAction, Single(action))` per `Action::ALL` in declaration
    /// order; override to introduce paired slots, route into `Nav`, or
    /// to omit data-dependent slots.
    fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Action>)> {
        Self::Action::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }

    /// Pane's current input mode. One of three variants:
    ///   - `Navigable`: list/cursor — `NavigationAction` drives it; framework
    ///      emits the `Nav` region.
    ///   - `Static`: no scrolling, no typed input (Output, Toasts,
    ///      KeymapPane Conflict). `Nav` suppressed; `Global` emitted.
    ///   - `TextInput`: pane consumes typed characters (Finder,
    ///      SettingsPane Editing, KeymapPane Awaiting). `Nav` and
    ///      `Global` regions both suppressed; structural Esc
    ///      pre-handler also suppressed.
    fn input_mode(&self, _ctx: &Ctx) -> InputMode { InputMode::Navigable }

    /// Optional vim-extras: pane actions that should also bind to a
    /// keybind when `VimMode::Enabled`. Default empty. Used by
    /// `ProjectListAction::ExpandRow` (binds `'l'`) /
    /// `CollapseRow` (binds `'h'`). `KeyBind` (not `char`) so future
    /// extras can include modifier keys.
    fn vim_extras() -> &'static [(Self::Action, KeyBind)] { &[] }

    /// Returns a free function the framework calls to dispatch an
    /// action. The function takes `&mut Ctx` so the framework holds
    /// the only `&mut` borrow during dispatch (split-borrow: framework
    /// cannot hold `&mut self` from inside `&mut Ctx` while also
    /// passing `&mut Ctx`). Each pane registers a free function:
    ///
    /// ```rust
    /// fn dispatch_package(action: PackageAction, app: &mut App) { … }
    /// impl Shortcuts<App> for PackagePane {
    ///     fn dispatcher() -> fn(Self::Action, &mut App) { dispatch_package }
    /// }
    /// ```
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}
```

A pane writes `impl Shortcuts<App> for PackagePane`.

- `SCOPE_NAME` — TOML table name; survives type renames; one-line cost. Required.
- `defaults` — pane's default bindings. No framework default (every pane has its own keys).
- `label(action, ctx) -> Option<&'static str>` — returns the bar label for an action; `None` hides the slot. Default impl returns `Some(action.bar_label())` (the static label declared in `action_enum!`); panes override only for state-dependent labels (CiRuns Activate at EOL hidden, Package Activate label = "open" when on `CratesIo` field).
- `state(action, ctx) -> ShortcutState` — `Enabled` (lit) or `Disabled` (grayed). Default `Enabled`; override when the action is visible but inert.
- `bar_slots(ctx)` — declares the slot layout per-frame. Most panes accept the default (one slot per action, declaration order). Panes with paired slots override to return `vec![BarSlot::Paired(NavUp, NavDown, "nav"), BarSlot::Single(Activate), …]`. Data-dependent omission (CiRuns toggle slot only when ci data is present) lives here too.
- `input_mode` — `InputMode::Navigable` / `Static` / `TextInput`. Gates Nav region (only when `Navigable`), Global strip (suppressed on `TextInput`), and structural Esc pre-handler (suppressed on `TextInput`).
- `vim_extras` — pane-action vim bindings (separate from `Navigation`'s arrow → vim mapping).
- `dispatcher` — returns a free function pointer. Framework calls `dispatcher()(action, ctx)`.

### `BarSlot` enum

```rust
pub enum BarSlot<A> {
    Single(A),                  // one action, full key list shown via display_short joined by ','
    Paired(A, A, &'static str), // two actions glued with `/`, one shared label, primary keys only
}
```

Framework rendering:
- `Single(action)` → renders all keys bound to `action` (joined by `,` after `display_short`) `<space>` `pane.label(action, ctx)`.
- `Paired(left, right, label)` → renders `display_short(left.primary) "/" display_short(right.primary) <space> label`. **Primary keys only — alternative bindings for paired actions never appear in paired slots.** Used for `↑/↓ nav`, `←/→ expand`, `+/- all`, `←/→ toggle`.

`KeyBind::display_short` for any key intended to render in a paired slot must not produce a string containing `,` or `/`. The framework `debug_assert!`s this in `Paired` rendering and a Phase 2 unit test walks every `KeyCode` variant via `display_short` to confirm.

### `Navigation` — declarative, single instance per app

```rust
pub trait Navigation<Ctx: AppContext> {
    type Action: ActionEnum + 'static;
    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Action;
    const DOWN:  Self::Action;
    const LEFT:  Self::Action;
    const RIGHT: Self::Action;
    fn defaults() -> Bindings<Self::Action>;

    /// Free function the framework calls when any navigation action fires.
    /// `focused` lets the app dispatch to whichever scrollable surface
    /// owns the focused pane. One match arm per action, mirroring the
    /// `Shortcuts::dispatcher` and `Globals::dispatcher` pattern.
    fn dispatcher() -> fn(Self::Action, FocusedPane<Ctx::AppPaneId>, &mut Ctx);
}
```

Pane scopes carry per-instance state and dispatch logic; their bar contribution depends on that state. `Navigation` has neither (no per-instance state) but it does need dispatch, since the focused pane needs to scroll on `Up`. The framework reads the four required consts to render the nav row and to apply vim-mode bindings, and calls `dispatcher()(action, focused, ctx)` to route.

### `Globals` — declarative, app extension scope

```rust
pub trait Globals<Ctx: AppContext> {
    type Action: ActionEnum + 'static;
    const SCOPE_NAME: &'static str = "global";
    fn render_order() -> &'static [Self::Action];
    fn defaults() -> Bindings<Self::Action>;
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}
```

The app's *additional* globals scope (Find, OpenEditor, Rescan, etc.). The framework's pane-management/lifecycle globals are owned separately by `GlobalAction` (below); the app does not redefine them.

---

## `Keymap<Ctx>` runtime container

> Formal API in `core-api.md` §6 (`Keymap<Ctx>`) and §7 (`KeymapBuilder<Ctx>`, including the type-state choice, `BuilderError` variants, required vs optional methods).

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
    .with_navigation::<AppNavigation>()
    .with_globals::<AppGlobals>()
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

When a single pane has multiple modes (Browse vs Editing for Settings, Browse vs Awaiting vs Conflict for Keymap), use an internal mode flag and route via `label()` / `state()` / `dispatch()`. Do **not** create a separate `*Pane` type per mode.

Mode-neutral action names (`Activate`, `Cancel`, `Left`, `Right`) describe the user's intent; the pane decides what each intent does in each mode:

```rust
impl Shortcuts<App> for SomePane {
    fn label(&self, action: SomeAction, _ctx: &App) -> Option<&'static str> {
        Some(match (self.mode, action) {
            (Mode::Browse, SomeAction::Activate) => "edit",
            (Mode::Edit,   SomeAction::Activate) => "confirm",
            // …
        })
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

Today `keymap_ui.rs:236-250` calls `is_navigation_reserved` during binding capture to reject Up/Down/etc. Post-refactor the framework's KeymapPane (in Awaiting mode) reads `keymap.scope_for::<NavigationAction>().action_for(&candidate_bind)` and rejects when `Some(_)`. Same for vim keys via `is_vim_reserved`. The hardcoded `NAVIGATION_RESERVED` and `VIM_RESERVED` tables disappear.

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

`is_legacy_removed_action` is dropped — `tui_pane` carries no legacy migration. If cargo-port needs to handle removed-action TOML keys, it does so before calling `load_toml`.

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
| `Nav` | nav row from `Navigation::UP/DOWN`, pane-cycle row from `GlobalAction::NextPane` | optional extra paired rows (ProjectList: `←/→ expand`, `+/- all`) | `input_mode == Navigable` |
| `PaneAction` | nothing | every pane's per-action rows | always |
| `Global` | `GlobalAction` strip + `AppGlobals::render_order()` | nothing | `input_mode != TextInput` |

A pane indicates *which region* each of its rows lands in. Most pane rows go into `PaneAction`; ProjectList is the rare case where a pane pushes paired rows into `Nav`.

### Framework panes

| Pane | `input_mode` | `bar_slots` |
|---|---|---|
| `KeymapPane` (Browse) | `Navigable` | `[(PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `KeymapPane` (Awaiting) | `TextInput` | `[(PaneAction, Single(Cancel))]` (user is capturing a keystroke) |
| `KeymapPane` (Conflict) | `Static` | `[(PaneAction, Single(Clear)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Browse) | `Navigable` | `[(Nav, Paired(ToggleBack, ToggleNext, "toggle")), (PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Editing) | `TextInput` | `[(PaneAction, Single(Confirm)), (PaneAction, Single(Cancel))]` |
| `Toasts` | `Static` | `[(PaneAction, Single(Dismiss))]` |

### Trait change

`Shortcuts::bar_slots` returns `(region, row)` pairs:

```rust
fn bar_slots(&self, ctx: &Ctx) -> Vec<(BarRegion, BarSlot<Self::Action>)> {
    Self::Action::ALL.iter()
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

`bar/mod.rs::render()` calls `pane.bar_slots(ctx)` once, reads `pane.input_mode(ctx)`, then walks `BarRegion::ALL` and dispatches:

- `BarRegion::Nav` → `nav_region::render(pane, ctx, keymap, &rows)` — emits framework's nav + pane-cycle rows plus any `(Nav, _)` rows from `rows`. Skipped unless `input_mode == Navigable`.
- `BarRegion::PaneAction` → `pane_action_region::render(pane, ctx, keymap, &rows)` — emits every `(PaneAction, _)` row, calling `pane.label(action, ctx)` and `pane.state(action, ctx)` to assemble the slot.
- `BarRegion::Global` → `global_region::render(keymap, framework)` — emits `GlobalAction` + `AppGlobals::render_order()`. Skipped when `input_mode == TextInput`.

Each region module returns `Vec<Span>`; `mod.rs` joins them left-to-right with framework-owned spacing into a single `StatusBar`.

## Bar architecture — framework-owned

The status bar is a framework feature. App authors write no bar layout code. See the `BarRegion` section above for the three-region model and the `bar/` module structure.

| Concern | Owner |
|---|---|
| Region orchestration | Framework — `bar/mod.rs` walks `BarRegion::ALL` |
| `Nav` region (paired rows from `Navigation` + pane-cycle from `GlobalAction::NextPane`) | Framework — `bar/nav_region.rs`; emitted only when `pane.input_mode(ctx) == Navigable` |
| `PaneAction` region | Framework — `bar/pane_action_region.rs`; emits `(PaneAction, _)` rows from `pane.bar_slots(ctx)`, calling `pane.label(action, ctx)` + `pane.state(action, ctx)` to assemble the slot |
| `Global` region (`GlobalAction` + `AppGlobals::render_order()`) | Framework — `bar/global_region.rs`; suppressed when `pane.input_mode(ctx) == TextInput` |
| Color / style / spacing | Framework |
| Per-action label & enabled state | Pane (via `Shortcuts::label` + `Shortcuts::state`; `label` defaults to `Some(action.bar_label())`) |
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

Pane is the focused one. Framework calls `pane.bar_slots(ctx)` once, then walks `BarRegion::ALL`: each region module filters for its own region tag and emits spans. Region rendering consults `pane.input_mode(ctx)` for suppression (`Nav` skipped unless `Navigable`; `Global` skipped on `TextInput`). Result is a single `StatusBar` value the binary draws to the frame.

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

New action enums for previously-hardcoded surfaces:

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

1. **Structural pre-handler** — `GlobalAction::Dismiss` when `app.has_dismissable_output()` is true *and* focus is not a text-input pane (today this is the Esc-clears-`example_output` path at `input.rs:112-119`). Gated on `focused_pane.input_mode() != InputMode::TextInput` so typed keys can't trigger structural dismiss while the user is typing into Finder.
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

### Phases 2–9 — `tui_pane` foundations

Phases 2–9 land the entire `tui_pane` public surface in dependency order, one mergeable commit per phase. The canonical type spec is `core-api.md` (sections referenced from each sub-phase below); §11 is the canonical module hierarchy and the public re-export set in `lib.rs`. Type detail per file is in `core-api.md` §§1–10.

**Strictly additive across Phases 2–9.** Nothing moves out of the binary in this group. The binary continues to use its in-tree `keymap_state::Keymap`, `shortcuts::*`, etc., untouched. The migration starts in Phase 12.

**Pre-Phase-2 precondition (post-tool-use hook).** Decide hook strategy before Phase 2 lands: repo-local override at `.claude/scripts/hooks/post-tool-use-cargo-check.sh` adding `--workspace`, vs. updating the global script at `~/.claude/scripts/hooks/post-tool-use-cargo-check.sh`. Without the flag, edits to `tui_pane/src/*.rs` from inside the binary working dir will not surface `tui_pane` errors. Repo-local override is the lower-blast-radius option.

**README precondition (Phase 9).** `tui_pane/README.md` lands at the end of Phase 9 — when the public API is complete. It covers crate purpose + a minimal example using `Framework::new(initial_focus)`. Code blocks in the README are ` ```ignore ` (no doctests in this crate).

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
- Modifier display order (`Ctrl` → `Alt` → `Shift`) and the case-preservation policy in `parse` (`"Ctrl+K"` → `Char('K')`, not `Char('k')`) are now baked into Phase 2 tests. Phase 8 (TOML loader) inherits both as facts; if the loader needs case-insensitive letter lookup, that is a *keymap-layer* normalization, not a `KeyBind::parse` concern.

**Implications for remaining phases:**
- `core-api.md` §1 is out of sync with shipped code (signatures + error variants). Update before any later phase reads it as canonical.
- Phase 8 (`Keymap<Ctx>` + TOML loader) must decide letter-case normalization policy explicitly — `parse` preserves case as-is.
- Future framework error types (`KeymapError` Phase 4 skeleton, fill in Phase 8) should use `#[derive(thiserror::Error)]` with `#[from] KeyParseError` for source chaining, per the pattern established here.

#### Phase 2 Review

- Phase 3: rename `keymap/traits.rs` → `keymap/action_enum.rs` so the file name matches its sole resident (`ActionEnum` + `action_enum!`) and does not collide with Phase 7's per-trait file split.
- Phase 4: `KeymapError` ships with `#[derive(thiserror::Error)]` + `#[from] KeyParseError` for source chaining, and unit tests are rescoped to constructs that exist by end of Phase 4 (vim-application test deferred to Phase 9). `bindings!` macro tests now cover composed `KeyBind::ctrl(KeyBind::shift('g'))`.
- Phase 8: loader explicitly lowercases single-letter TOML keys (so `quit = "Q"` binds `Char('q')`); modifier display order is canonical `Ctrl+Alt+Shift+key` (no round-trip ordering preservation); vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods); `KeymapError` source chain from `KeyParseError` is asserted.
- Phase 11: paired-row separator policy made explicit — `Paired::debug_assert!` covers only the parser-producible `KeyCode` set; exotic variants may panic, and widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep.
- Phase 12: `anyhow = "1"` lands in the binary's `Cargo.toml` here (first call site that needs context wrapping is `Keymap::<App>::builder(...).load_toml(?)?.build()?`).
- §1 (`Pane id design`): `PaneId` → `FrameworkPaneId` everywhere, including the inside-the-crate short form, so the type name is one-to-one across library and binary call sites.
- `core-api.md` §1 + `tui-pane-lib.md` §11 lib.rs sketch synced to shipped Phase 2 code: `shift`/`ctrl` take `impl Into<Self>`, `From<KeyEvent>` documented, 3-variant `KeyParseError` (`InvalidChar` dropped), parser policy (`"Control"` synonym, `"Space"` token, case-preserving) called out.
- `toml-keys.md` synced to the Zed/VSCode/Helix-aligned letter-case decision: loader lowercases single-letter ASCII keys (`"Q"` → `Char('q')`, never `Shift+q`); modifier tokens are case-insensitive on input but writeback canonical capitalized; named-key tokens (`Enter`, `Tab`, `F1`, …) are case-sensitive with no aliases; non-ASCII letters not lowercased; modifier repeats silently OR'd (not rejected — bitwise OR is idempotent).
- Phase 6 + Phase 10 now spell out the **Phase 6 → Phase 10 contract**: Phase 6 freezes a 1-field / 3-method `Framework<Ctx>` skeleton (`focused` field, `new`/`focused()`/`set_focused()`); Phase 10 is purely additive on top. Mirrored at both phase blocks so neither side can drift independently.
- Decided: `KeyEvent` press/release/repeat handling uses a typed wrapper enum (`KeyInput { Press, Release, Repeat }`) at the framework boundary, not a runtime check at each dispatch site and not a fallible `Option`-returning conversion. Modeled after Zed/GPUI's typed-event split. Repeat is preserved (not collapsed into Press) so future handlers can opt into auto-repeat behavior. Phases 13–15 dispatch sites pattern-match `KeyInput::Press(bind)` (or call `.press()`); the event-loop entry produces `KeyInput` once.

### Phase 3 — Action machinery ✅

Add `tui_pane/src/keymap/action_enum.rs` with `ActionEnum` + `action_enum!` (per §4 — the trait part; the three scope traits land in Phase 7). Add `tui_pane/src/keymap/global_action.rs` with `GlobalAction` and its `ActionEnum` impl (§10). Add `tui_pane/src/keymap/vim.rs` with `VimMode::{Disabled, Enabled}` (§10).

> File `action_enum.rs` (not `traits.rs`) and `global_action.rs` (not `base_globals.rs`) — the file name matches the contained type. The three scope traits live in their own files (`shortcuts.rs` / `navigation.rs` / `globals.rs`) per Phase 7.

#### Retrospective

**What worked:**
- Three-file split (`action_enum.rs` / `global_action.rs` / `vim.rs`) lined up one-to-one with shipped code — no scope drift. 12 unit tests cover macro expansion (`action_enum!` against a fixture `Foo` enum) and the hand-rolled `GlobalAction` impl. Workspace clippy clean under `pedantic` + `nursery` + `all` + `cargo`.
- `pub use keymap::ActionEnum;` at crate root in `lib.rs` keeps the macro's `$crate::ActionEnum` path stable regardless of the trait's true module location. The macro can be re-homed later without breaking any expansion site.
- `VimMode` defaults to `Disabled` via `#[derive(Default)]` + `#[default]` on the variant — no hand-written `Default` impl needed.

**What deviated from the plan:**
- Hand-rolled `impl Display for GlobalAction` (delegates to `description()`) — not strictly required by the spec but mirrors what the macro generates for `action_enum!`-produced enums, so all `ActionEnum` impls render the same way under `format!("{action}")`. Cost: 4 lines.
- `crate::ActionEnum` (root re-export) is the trait path used inside `global_action.rs`'s test module rather than a longer `super::super::action_enum::ActionEnum` — single-`super::` is fine in normal code, double-`super::` is banned by project policy.

**Surprises:**
- `clippy::too_long_first_doc_paragraph` (nursery) fires on multi-sentence module headers. `global_action.rs`'s opening `//!` block had to be split into a one-line summary + blank `//!` + body. Likely to fire elsewhere when later phases ship docs. No code change required, but worth knowing for module-doc authoring.
- The `from_toml_key` returning `Option<Self>` (not `Result`) is intentional and the trait method has no scope context to attach. The TOML loader (Phase 4 skeleton, Phase 8 fill) lifts `None` into `KeymapError::UnknownAction { scope, action }`. Recorded explicitly here so Phase 4/8 don't accidentally widen the trait.

**Implications for remaining phases:**
- `macros.md` §3 still references `crate::keymap::traits::ActionEnum` — needs to be `crate::keymap::action_enum::ActionEnum`. Synced as part of this review.
- `core-api.md` §10 `KeymapError` snippet has `impl Display { /* … */ }` placeholder — Phase 4 lands the real impl via `#[derive(thiserror::Error)]` per the Phase 2 retrospective decision.
- Phase 4 (`bindings!` macro) follows the same `#[macro_export] macro_rules!` declaration template used here; the doctest pattern can mirror Phase 3's approach (`crate::action_enum! { … }` inside an internal `mod tests`).
- Phase 12 (binary swap to `tui_pane::action_enum!`): seven existing `action_enum!` invocations in `src/keymap.rs` swap to the `tui_pane::` prefix; the macro's grammar is identical, so each invocation needs only the prefix change.

#### Phase 3 Review

Architectural review of remaining phases (4-17) returned 18 findings — 13 minor (applied directly), 5 significant (decided with the user). Resolved outcomes:

- **Renamed `keymap/base_globals.rs` → `keymap/global_action.rs`** so the file name matches the contained type (`GlobalAction`). User did the file rename in their editor; doc references and `mod.rs` synced. No `BaseGlobals` type ever existed; the "base" prefix earned nothing and broke the established `key_bind.rs → KeyBind` convention.
- **Phase 8 anchor type:** `Keymap<Ctx>` lives in `keymap/mod.rs` (option c). Workspace lint `self_named_module_files = "deny"` rules out `keymap.rs` + `keymap/` sibling layout, and `clippy::module_inception` rules out `keymap/keymap.rs`. Phase 6 already follows the same convention with `framework/mod.rs` holding `Framework<Ctx>`. Plan's prior `keymap/mod_.rs` was a typo.
- **Framework owns `GlobalAction` dispatch (significant pivot, item 2):** `KeymapBuilder` no longer takes positional `(quit, restart, dismiss)` callbacks. Framework dispatches all seven variants:
  - `Quit` / `Restart` set `Framework<Ctx>::quit_requested` / `restart_requested` flags; binary's main loop polls.
  - `Dismiss` runs framework chain (toasts → focused framework overlay), then bubbles to optional `dismiss_fallback`.
  - `NextPane` / `PrevPane` / `OpenKeymap` / `OpenSettings` framework-internal as before.
  - Binary opts in via optional `.on_quit()` / `.on_restart()` / `.dismiss_fallback()` chained methods on `KeymapBuilder`.
  - Rationale: hit-test for the mouse close-X on framework overlays already lives in the framework. Splitting Esc-key dismiss between framework (overlays) and binary (everything else) duplicates that ownership.
  - Touches Phase 6 (Framework skeleton +2 fields, +2 methods), Phase 9 (KeymapBuilder drops 3 args, gains 3 chained hooks), Phase 10 (Toasts dismiss participation, `Framework::dismiss()` method), Phase 16 (binary main loop polls flags, deletes `Overlays::should_quit`).
- **Cross-enum `[global]` collision = hard error (item 3):** `KeymapError::CrossEnumCollision { key, framework_action, app_action }` at load time. Definition-time error — app dev renames their colliding `AppGlobalAction::toml_key` string. Per-binding revert policy still handles user typos.
- **`GlobalAction::defaults()` lives on the enum (item 4):** `pub fn defaults() -> Bindings<Self>` lands in Phase 4 (when `Bindings` + `bindings!` exist) inside `global_action.rs`. Loader and builder consume it.
- **Cross-crate macro integration test (item 5):** `tui_pane/tests/macro_use.rs` lands as a Phase 3 follow-up — exercises `tui_pane::action_enum!` from outside the crate. Phase 4 extends it for `tui_pane::bindings!`.

Minor findings applied directly (no user gating):
- Stale `keymap::traits::ActionEnum` references in `macros.md` synced to `keymap::action_enum::ActionEnum`.
- Phase 4 root re-exports (`Bindings`, `KeyBind`) called out for the `bindings!` macro's `$crate::` paths.
- `KeymapError` variant set spelled out in Phase 4 + `core-api.md` §10 (with `#[derive(thiserror::Error)]`).
- `Shortcuts::vim_extras() -> &'static [(Self::Action, KeyBind)]` called out in Phase 7 with default `&[]`.
- Vim-mode skip-already-bound test moved Phase 8 → Phase 9 (vim application is the builder's job per "Vim mode — framework capability" §).
- `AppContext::AppPaneId: Copy + Eq + Hash + 'static` super-trait set added to Phase 6 (required by Phase 10's `HashMap<AppPaneId, fn(&Ctx) -> InputMode>`).
- `core-api.md` §4 `ActionEnum` super-trait set synced to shipped code (`Copy + Eq + Hash + Debug + Display + 'static` — adds `Debug` + `Display`).
- Phase 8 explicit "loader lifts `None` from `from_toml_key` into `KeymapError::UnknownAction`" wording added.
- `clippy::too_long_first_doc_paragraph` (nursery) guidance added to the per-phase rustdoc precondition.
- `pub use keymap::GlobalAction;` at crate root noted in Phase 12.
- Paired-row separator policy in Phase 11 shortened to a one-line cross-reference of Phase 2's locked decision.

### Phase 4 — Bindings, scope map, loader errors ✅

Add `tui_pane/src/keymap/bindings.rs` (`Bindings<A>` + `bindings!`, §2), `tui_pane/src/keymap/scope_map.rs` (`ScopeMap<A>`, §3), and `tui_pane/src/keymap/load.rs` skeleton holding `KeymapError` (§10). The loader's actual TOML-parsing impl lands in Phase 8 alongside `Keymap<Ctx>`.

**Also lands in Phase 4 (post-Phase-3 review):** `pub fn defaults() -> Bindings<Self>` on `GlobalAction` in `tui_pane/src/keymap/global_action.rs` — returns the canonical `q` / `R` / `Tab` / `Shift+Tab` / `Ctrl+K` / `s` / `x` bindings using the `bindings!` macro that ships in this phase. Co-located with the enum (matches the convention every `Shortcuts<P>::defaults()` impl follows). Tested in `global_action.rs` directly; loader and builder consume it.

**Root re-exports (shipped form).** As implemented, `tui_pane/src/lib.rs` is `mod keymap;` (private) plus crate-root `pub use` for every public type: `ActionEnum`, `Bindings`, `GlobalAction`, `KeyBind`, `KeyInput`, `KeyParseError`, `KeymapError`, `ScopeMap`, `VimMode`. The speculative `pub use keymap::bindings::bindings;` from earlier drafts proved unnecessary — `#[macro_export]` already places the macro at the crate root.

`KeymapError` is `#[derive(thiserror::Error)]` and ships with seven variants (the loader and builder consume them in Phases 8 and 9):
- `Io(#[from] std::io::Error)` — file-open failure.
- `Parse(#[from] toml::de::Error)` — top-level TOML parse failure.
- `InArrayDuplicate { scope, action, key }` — duplicate key inside one TOML array.
- `CrossActionCollision { scope, key, actions: (String, String) }` — same key bound to two actions.
- `InvalidBinding { scope, action, #[source] source: KeyParseError }` — `KeyBind::parse` failure with chained source.
- `UnknownAction { scope, action }` — `A::from_toml_key(key)` returned `None`; loader attaches the scope.
- `UnknownScope { scope }` — TOML referenced an unknown top-level table.

Phase 4 ships the `enum` definition; Phase 8 wires the actual loader paths that emit each variant. (`BuilderError` from Phase 9 is a separate enum at the builder layer — see Phase 9.)

`bindings!` macro grammar must accept arbitrary `impl Into<KeyBind>` expressions on the RHS — including composed forms like `KeyBind::ctrl(KeyBind::shift('g'))` (CTRL|SHIFT, established by Phase 2). The macro's unit tests cover the composed case.

**Cross-crate macro integration test.** Extend `tui_pane/tests/macro_use.rs` (the scaffolding lands as a Phase 3 follow-up exercising `action_enum!` only) to add a `bindings!` invocation. Both macros are compiled here from outside the defining crate — `#[macro_export]` + `$crate::` paths are easy to break under cross-crate use, and this test locks the public path before Phase 12's binary swap depends on it.

Unit tests (this phase, scoped to what exists by end of Phase 4):
- `Bindings::insert` preserves insertion order; first key for an action is the primary.
- `ScopeMap::add_bindings` on an empty map produces `by_key.len() == by_action.values().map(Vec::len).sum::<usize>()` (no orphan entries).
- `bindings!` accepts `KeyBind::ctrl(KeyBind::shift('g'))` and stores `KeyModifiers::CONTROL | SHIFT`.
- (Deferred to Phase 9, when the builder + `VimMode::Enabled` application pipeline exist:) `MockNavigation::Up` keeps its primary as the inserted `KeyCode::Up` even with `VimMode::Enabled` applied — insertion-order primary.

#### Retrospective

**What worked:**
- `bindings!` macro grammar (`KEY => ACTION` and `[KEYS] => ACTION` arms with optional trailing commas) accepted every authoring case the test suite threw at it, including `KeyBind::ctrl(KeyBind::shift('g'))` composed modifiers.
- `tests/macro_use.rs` cross-crate test caught a `$crate::*` path break the moment we flipped `pub mod keymap` → `mod keymap` (cross-crate paths started failing immediately, before any consumer noticed).
- 49 tui_pane tests pass; 599 workspace tests pass; `cargo mend --fail-on-warn` reports no findings.

**What deviated from the plan:**
- **`pub mod` removed everywhere.** Plan said "extend root re-exports for `Bindings`, `KeyBind`." Per `cargo mend` (which denies `pub mod` workspace-wide) and direct user instruction, `tui_pane/src/lib.rs` was reduced to `mod keymap;` (private) plus crate-root `pub use` for every public type: `ActionEnum`, `Bindings`, `GlobalAction`, `KeyBind`, `KeyInput`, `KeyParseError`, `KeymapError`, `ScopeMap`, `VimMode`. `keymap/mod.rs` similarly switched all `pub mod foo;` to `mod foo;` + facade `pub use`. **Public-API change:** the `tui_pane::keymap::*` namespace no longer exists — every type is now flat at `tui_pane::*`.
- **`bindings!` macro is a two-step expansion.** Spec'd as a single `macro_rules!` with one block-returning arm. A single arm cannot recurse to handle mixed `KEY => ACTION` / `[KEYS] => ACTION` lines, so the macro now delegates to a `#[doc(hidden)] #[macro_export] macro_rules! __bindings_arms!` incremental TT muncher. Public surface unchanged; `__bindings_arms!` is the implementation detail.
- **`ScopeMap::new` / `insert` are `pub(super)`, not `pub(crate)`.** The design doc said `pub(crate)`; project memory `feedback_no_pub_crate.md` (use `pub(super)` in nested modules — `pub(crate)` reserved for top-level files) overruled. Same author intent (framework-only construction), narrower scope.
- **`bind_many` requires `A: Clone`, not just `A: Copy`.** The loop body needs to clone the action per key; `Copy` only matters when the entire `Bindings` is consumed. Trivial in practice — every `ActionEnum` is `Copy + Clone`.
- **`bindings!` uses `$crate::KeyBind`, not `$crate::keymap::KeyBind`.** Falls out of the `pub mod keymap` removal: the macro's `$crate::*` paths now reach the flat root re-exports.

**Surprises:**
- **clippy `must_use_candidate` (pedantic) fires on every getter.** Each new public method that returns a value needs `#[must_use]`. Apply pre-emptively in Phase 5+.
- **`cargo mend` denies `pub mod` workspace-wide and there is no `mend.toml` allowlist.** Phases 5–11 must declare every new module as private `mod foo;` plus `pub use foo::Type;` at the parent facade — never `pub mod foo;`.
- **`src/tui/panes/support.rs` had three pre-existing mend warnings** (inline path-qualified types) that auto-resolved during the Phase 4 build cycle — picked up "for free." Not part of Phase 4 scope but landed in the same diff.

**Implications for remaining phases:**
- **Every Phase 5+ module declaration must be `mod foo;`** (not `pub mod foo;`) at every level. Affects Phase 5 (`bar/region.rs`, `bar/slot.rs`), Phase 6 (`framework/`), Phase 7 (scope traits), Phase 8 (`keymap/container.rs` or wherever `Keymap<Ctx>` lands), Phase 9 (`keymap/builder.rs`), Phase 10 (`panes/*`), Phase 11 (`bar/render.rs`).
- **Every `tui_pane::keymap::*` path in design docs is now stale.** `phase-02-core-api.md`, `phase-02-macros.md`, `phase-02-test-infra.md`, `phase-02-toml-keys.md`, and the rest of `tui-pane-lib.md` need a sweep: `crate::keymap::Foo` → `crate::Foo` (and `tui_pane::keymap::Foo` → `tui_pane::Foo` in public-API examples).
- **Phase 12 binary swap uses flat paths.** `use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`. Every file in `src/tui/` that touches keymap types will see this.
- **`pub(super)` is the visibility default for framework-internal construction.** Phase 8's `Keymap<Ctx>` constructor, Phase 9's `KeymapBuilder::build()` — apply the same rule: `pub(super)` for sites only the framework's own `keymap/` siblings call.
- **Pre-emptive `#[must_use]` on every Phase 5+ public getter** saves a clippy round-trip per phase.

#### Phase 4 Review

- **Phase 4 plan text reconciled** with shipped `KeymapError` (added `Io(#[from])` and `Parse(#[from] toml::de::Error)` — the previous variant list of 5 omitted them).
- **Stale "Extend root re-exports" paragraph rewritten** to reflect the shipped lib.rs (every public type re-exported flat at crate root; no `pub use keymap::bindings::bindings;`).
- **Phase 5 (Bar primitives)** gains an explicit "Root re-exports" line: `lib.rs` adds `pub use bar::{BarRegion, BarSlot, InputMode, ShortcutState};`. The `Shortcut` wrapping struct is gone — `Shortcuts::label` returns `Option<&'static str>` directly and `Shortcuts::state` returns `ShortcutState`.
- **Phase 6 (Framework skeleton)** gains an explicit "Root re-exports" line: `pub use framework::Framework;`, `pub use pane_id::{FocusedPane, FrameworkPaneId};`, `pub use app_context::AppContext;` plus `#[must_use]` directive.
- **Phase 7 (Scope traits)** gains: `pub use keymap::{Shortcuts, Navigation, Globals};` plus standing-rule 1 reminder.
- **Phase 8 (Keymap container)** gains: `pub use keymap::Keymap;`, `pub(super)` for `Keymap::new`, `#[must_use]` on getters.
- **Phase 9 (Keymap builder)** gains: `pub use keymap::{KeymapBuilder, BuilderError};`, `pub use settings::SettingsRegistry;`, `pub(super)` for builder internals.
- **Phase 10 (Framework panes)** gains: `pub use panes::{KeymapPane, SettingsPane, Toasts};`, panes/mod.rs declared `mod` (private) per standing rule 1.
- **Phase 11 (Bar renderer)** gains: `pub use bar::StatusBar;` plus standing-rule 1 reminder.
- **Phase 12 (App swap)** gains: flat-namespace import note (`use tui_pane::KeyBind;` not `use tui_pane::keymap::KeyBind;`) and binary-side `mod` rule reminder.
- **New "Phase 5+ standing rules" subsection** added after the Phase 4 retrospective: locks the seven standing rules (private `mod`, flat re-exports, `pub(super)` for framework-internal, `#[must_use]` on getters, flat `$crate::*` macro paths, new `#[macro_export]` extends `tests/macro_use.rs`, `cargo mend --fail-on-warn` as phase-completion gate).
- **Definition of done** rewritten to enumerate every public type at crate-root flat paths and to call out `__bindings_arms!` as `#[doc(hidden)]` but technically reachable.
- **Spec docs swept** (`core-api.md`, `macros.md`): stale `crate::keymap::<submod>::Foo` sub-paths replaced with the facade-path form `crate::keymap::{Foo, ...}`; explanatory comments added about why the public API is flat.
- **Reviewed and not changed:** `tui_pane/README.md` deferred to Phase 16 (subagent finding #20 — no earlier baseline justified). `bind_many` requiring `A: Clone` (subagent finding #10 — auto-satisfied because `ActionEnum: Copy`, no plan change needed).

These apply to every remaining phase without further mention; phase blocks below assume them. Restate only where a phase has a specific exception.

1. **Module declarations are `mod foo;`** at every level — never `pub mod foo;`. Parents expose the API via `pub use foo::Type;` re-exports. `cargo mend` denies `pub mod` workspace-wide, including the binary side (`src/tui/...` in Phase 12).
2. **Public types live at the crate root.** Every `tui_pane` public type re-exports from `tui_pane/src/lib.rs` so callers write `tui_pane::Foo` (flat). The `tui_pane::keymap::*` namespace does not exist publicly.
3. **Framework-internal construction is `pub(super)`.** New / insert / build methods that only the framework's own siblings call use `pub(super)`, never `pub(crate)`. Project memory `feedback_no_pub_crate.md` for rationale.
4. **Public getters get `#[must_use]` pre-emptively.** Clippy `must_use_candidate` (pedantic, denied) fires on every getter that returns a value the caller can ignore.
5. **Macros use flat `$crate::*` paths.** Every `#[macro_export]` macro references re-exported root types: `$crate::Bindings`, `$crate::KeyBind`, `$crate::ActionEnum`. Never `$crate::keymap::Foo`.
6. **New `#[macro_export]` extends `tests/macro_use.rs`.** Cross-crate path stability is locked by that file; any new exported macro adds an invocation there.
7. **Phase-completion gates.** `cargo build`, `cargo nextest run`, `cargo +nightly fmt`, `cargo clippy --workspace --all-targets`, `cargo mend --fail-on-warn` — all clean before the phase is marked ✅.

### Phase 5 — Bar primitives ✅

Add `tui_pane/src/bar/region.rs` (`BarRegion::{Nav, PaneAction, Global}` + `ALL`), `tui_pane/src/bar/slot.rs` (`BarSlot<A>` + `ShortcutState`), and `InputMode` in `bar/mod.rs`. All per §5.

Phase 5 also amends Phase 3's `ActionEnum` trait to add `fn bar_label(self) -> &'static str` and extends the `action_enum!` macro grammar to take a tuple of three string literals per arm:

```rust
action_enum! {
    pub enum PackageAction {
        Activate => ("activate", "activate", "Open / activate selected field");
        Clean    => ("clean",    "clean",    "Clean target dir");
        //          ↑ TOML key   ↑ bar label ↑ keymap-UI description
    }
}
```

Leaf types only — the renderer that consumes them lands in Phase 11.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::{BarRegion, BarSlot, InputMode, ShortcutState};`. `bar/mod.rs` is `mod region; mod slot; pub use region::BarRegion; pub use slot::{BarSlot, ShortcutState}; pub use ...InputMode;` (or wherever `InputMode` lands).

**No `Shortcut` wrapping struct.** Phase 7's `Shortcuts<Ctx>` trait splits the bar-entry payload across two methods: `fn label(&self, action, ctx) -> Option<&'static str>` (default `Some(action.bar_label())`, `None` hides the slot) and `fn state(&self, action, ctx) -> ShortcutState` (default `Enabled`). The label string lives in exactly one return value; enabled/disabled is a separate orthogonal axis.

**`action_enum!` grammar amendment.** The macro arm changes from `Variant => "key", "desc";` to `Variant => ("key", "bar", "desc");`. Phase 3's existing `action_enum!` invocations in the keymap module and the `tests/macro_use.rs` smoke test must be updated in this phase. The 12-arm cargo-port migration in Phase 12 inherits the new grammar. The hand-rolled `GlobalAction` `ActionEnum` impl shipped in Phase 3 also needs a `bar_label()` method body — one match arm per variant (`Quit => "quit"`, `Restart => "restart"`, etc.).

**`Globals::bar_label` removed.** With `ActionEnum::bar_label` available on every action enum, the redundant `fn bar_label(action: Self::Action) -> &'static str` method on the `Globals<Ctx>` trait (Phase 7) is dropped. Bar code calls `action.bar_label()` regardless of which scope the action came from.

**No pre-existing call sites for `Shortcuts::label` / `state`.** The `Shortcuts<Ctx>` trait itself lands in Phase 7, so Phase 5 has nothing to migrate beyond the `action_enum!` arms. `tests/macro_use.rs` extends with a smoke test constructing `tui_pane::BarSlot::Single(...)`, `tui_pane::BarRegion::Nav`, and `tui_pane::ShortcutState::Enabled` from outside the crate to lock the flat-namespace public path.

#### Retrospective

**What worked:**
- `bar/region.rs`, `bar/slot.rs`, `bar/mod.rs` landed as flat `mod`-private files with crate-root re-exports — standing rules 1 + 2 applied without friction.
- Macro grammar change to `Variant => ("toml_key", "bar_label", "description");` was a single `macro_rules!` arm edit; both the inline `Foo` test enum and `tests/macro_use.rs` migrated trivially.
- Cross-crate test (`bar_primitives_reachable_from_outside_crate`) caught the public path before any consumer needed it — `tui_pane::BarSlot::Single`, `tui_pane::BarRegion::ALL`, `tui_pane::ShortcutState::Enabled`, `tui_pane::InputMode::Navigable` all reachable.
- 59 tui_pane tests pass; 659 workspace tests pass; clippy + mend clean.

**What deviated from the plan:**
- **Doc backticks needed on `BarRegion` variant references in `InputMode` docstrings.** Pedantic clippy `doc_markdown` flagged `PaneAction` mid-doc; wrapped `Nav`/`PaneAction`/`Global` in backticks. Standing rule 4 (`#[must_use]`) is the per-getter form of this same broader pedantic-clippy posture; bar primitives have no getters, so #4 didn't apply this phase.
- **`GlobalAction::bar_label` strings chosen explicitly.** Plan said "match arms per variant (`Quit => "quit"`, etc.)" without committing the full set. Shipped: `quit`, `restart`, `next`, `prev`, `keymap`, `settings`, `dismiss` — short forms for `NextPane`/`PrevPane`/`OpenKeymap`/`OpenSettings` (the `Open` prefix and `Pane` suffix are bar noise).

**Surprises:**
- **`bar_label` shorter than `toml_key` for `GlobalAction`.** Pattern: `toml_key = "open_keymap"`, `bar_label = "keymap"`, `description = "Open keymap viewer"`. Three-axis labelling (config-stable / bar-terse / human-readable) is the value the macro grammar buys us; the example arms in the plan all happened to use identical `toml_key`/`bar_label`, masking this.

**Implications for remaining phases:**
- **Phase 7 `Shortcuts::label` default body is one line.** `fn label(&self, action: Self::Action, _ctx: &Ctx) -> Option<&'static str> { Some(action.bar_label()) }` — `ActionEnum::bar_label` is implemented on every action enum (macro + the hand-rolled `GlobalAction`), so the trait default has zero per-impl boilerplate.
- **Phase 7 `Shortcuts::state` default is `ShortcutState::Enabled`.** Same: zero per-impl boilerplate.
- **Phase 12 cargo-port `action_enum!` migrations need the third positional string.** Every existing app-side invocation gains a bar label between the toml key and description. For app actions where the bar text matches the toml key, just duplicate the literal — no design decision per arm.
- **Phase 11 bar renderer reads `BarRegion::ALL` for layout order.** Already reflected in trait def — `Vec<(BarRegion, BarSlot<Self::Action>)>` returned, renderer groups by region.
- **No new public types added to `tui_pane::*` beyond the four announced** (`BarRegion`, `BarSlot`, `InputMode`, `ShortcutState`). Every later-phase reference to `tui_pane::Shortcut` (the deleted wrapping struct) is dead — caught any in Phase 5's plan-doc sweep, but Phase 7 implementers should not pattern-match on `Shortcut` in muscle memory.

#### Phase 5 Review

- **Phase 7 (Scope traits)** plan body now enumerates the full `Shortcuts<Ctx>` method set (cross-references `core-api.md` §4) and explicitly states the `label` / `state` default bodies leveraging `ActionEnum::bar_label` and `ShortcutState::Enabled`.
- **Phase 7** also explicitly states `Globals<Ctx>` has no `bar_label` method, and adds a `Shortcut` (singular wrapping struct) doc-grep step to confirm zero residue.
- **Phase 8 (Keymap container)** plan gains a one-line clarification that `bar_label` is code-side only — the TOML loader never reads or writes it.
- **Phase 11 (Bar renderer)** plan now states the per-region `InputMode` suppression rules in line with shipped `bar/mod.rs` docstrings (Static suppresses `Nav`, `TextInput` suppresses Nav + PaneAction + Global).
- **Phase 12 (App swap)** gains an explicit migration-cost callout that every existing `action_enum!` invocation in `src/tui/` needs a third positional `bar_label` literal.
- **Phase 17 (Regression tests)** reworded to assert each global slot's bar text comes from `action.bar_label()`, not a `Globals` trait method.
- **Doc-spec sync (`core-api.md`):** `ScopeMap::new`/`insert` migrated from `pub(crate)` → `pub(super)` to match shipped code (Phase 4 retrospective decision; finalized here per post-phase doc-sync rule).
- **Reviewed and not changed:** `Globals::render_order` (subagent finding #6 — already declared at `core-api.md:423`, plan unchanged); binary-side `pub mod` audit in Phase 12 (subagent finding #11 — grep of `src/tui/**/*.rs` found zero `pub mod`, no audit needed); `__bindings_arms!` cross-crate test (subagent finding #10 — `#[doc(hidden)]` is supported-surface-out, not worth dedicated test); `set_focused` consistency (subagent finding #4 — already consistent); Phase 9 builder-level cross-crate test (subagent finding #15 — Phase 9 already lists end-to-end builder tests).

### Phase 6 — Pane identity, ctx, Framework skeleton

The chicken-and-egg unit. `AppContext::framework()` returns `&Framework<Self>` and `Framework<Ctx>` requires `Ctx: AppContext`, so they must land together. `AppContext::set_focus` takes `FocusedPane<Self::AppPaneId>`, so the pane-id types come along.

Add:

- `tui_pane/src/pane_id.rs` — `FrameworkPaneId::{Keymap, Settings, Toasts}`, `FocusedPane<AppPaneId>::{App, Framework}`.
- `tui_pane/src/app_context.rs` — `AppContext` trait (`type AppPaneId: Copy + Eq + Hash + 'static`, `framework`, `framework_mut`, `set_focus`). The `AppPaneId` super-trait set mirrors `ActionEnum` (Phase 3) and is required by Phase 10's `HashMap<Ctx::AppPaneId, fn(&Ctx) -> InputMode>` registry.
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

> **Phase 6 → Phase 10 contract.** This 3-field / 5-method API is **frozen at Phase 6 and must survive Phase 10 verbatim.** Phase 10 is purely additive: it adds the `keymap_pane` / `settings_pane` / `toasts` fields, the `input_mode_queries` / `editor_target_path` / `focused_pane_input_mode` plumbing, the `dismiss()` method (framework dismiss chain), and any new query methods — but it **never renames** the five frozen methods or the three frozen fields. Tests written in Phases 7–9 against this surface stay green when Phase 10 lands.

No pane fields, no `input_mode_queries`, no `editor_target_path`, no `focused_pane_input_mode` in Phase 6 — those land in Phase 10 once framework panes exist.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use framework::Framework;`, `pub use pane_id::{FocusedPane, FrameworkPaneId};`, `pub use app_context::AppContext;`. Apply rule 4 (`#[must_use]`) to every getter on `Framework<Ctx>`.

### Phase 7 — Scope traits

Split §4 into one file per trait (each is independent, the heaviest is `Shortcuts<Ctx>` with 10+ items):

- `tui_pane/src/keymap/shortcuts.rs` — `Shortcuts<Ctx>`. Method set per `core-api.md` §4: `defaults`, `label`, `state`, `bar_slots`, `input_mode`, `input_mode_query`, `vim_extras`, `dispatcher`, plus `SCOPE_NAME` and `APP_PANE_ID` consts. `vim_extras() -> &'static [(Self::Action, KeyBind)]` defaults to `&[]` (per-pane vim-mode extras consumed by the builder in Phase 9; cargo-port's `ProjectListAction` overrides for `'l'`/`'h'` in Phase 12). Default `label` returns `Some(action.bar_label())` and default `state` returns `ShortcutState::Enabled` — both leverage Phase 5's `ActionEnum::bar_label` and the orthogonal-axis `ShortcutState` enum, so per-pane impls override only when label/state is state-dependent.
- `tui_pane/src/keymap/navigation.rs` — `Navigation<Ctx>`.
- `tui_pane/src/keymap/globals.rs` — `Globals<Ctx>` (app-extension globals, separate from the framework's own `GlobalAction` from Phase 3). The trait has **no** `bar_label(action) -> &'static str` method — Phase 5's `ActionEnum::bar_label` (live on every action enum, including the macro-generated and the hand-rolled `GlobalAction`) is the single source. Bar code calls `action.bar_label()` regardless of scope.

`keymap/action_enum.rs` (added in Phase 3) keeps `ActionEnum` + `action_enum!` only.

**`Shortcut` wrapping struct is dead.** Phase 5 collapsed it into the orthogonal `Option<&'static str>` (label) + `ShortcutState` (axis) split. Phase 7 prep verifies `core-api.md` and `paneid-ctx.md` reference no `Shortcut\b` (singular wrapping struct) — `Shortcuts`, `ShortcutState` are the only valid forms.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use keymap::{Shortcuts, Navigation, Globals};`. `keymap/mod.rs` adds `pub use shortcuts::Shortcuts; pub use navigation::Navigation; pub use globals::Globals;`. Inner files declare `mod shortcuts; mod navigation; mod globals;` (private — standing rule 1).

### Phase 8 — Keymap container

Add `Keymap<Ctx>` in `tui_pane/src/keymap/mod.rs` (the keymap module's anchor type lives in its `mod.rs` file, mirroring the Phase 6 precedent of `Framework<Ctx>` in `framework/mod.rs`). `Keymap<Ctx>` exposes `scope_for` / `navigation` / `globals` / `framework_globals` / `config_path` (per §6). Fill in the actual TOML-parsing implementation in `keymap/load.rs` (skeleton + `KeymapError` from Phase 4). Construction is via `Keymap::builder()` (no positionals — the framework owns `GlobalAction` dispatch, see Phase 3 review for full rationale); the builder itself lands in Phase 9.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use keymap::Keymap;`. The `Keymap::new`-style internal constructor that the builder calls is `pub(super)` (standing rule 3 — framework-only construction). Apply `#[must_use]` (standing rule 4) to every getter on `Keymap<Ctx>`.

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
- `KeyParseError` from `KeyBind::parse` chains into `KeymapError` via `#[from]` — round-trip a malformed binding string and assert the source error is preserved (`err.source().is_some()`).
- Unknown action in TOML (e.g. `[project_list] activte = "a"`) surfaces `KeymapError::UnknownAction { scope: "project_list", action: "activte" }` — the loader calls `A::from_toml_key(key)` and lifts `None` into the error variant with the scope name attached. Trait method stays as `Option<Self>` (no scope context); error context lives at the loader.

Vim-mode handling moved to Phase 9 (see "Vim mode — framework capability" §): vim binds are applied **inside `KeymapBuilder::build()`**, not the loader. Phase 8's loader is vim-agnostic.

### Phase 9 — Keymap builder + settings registry

Two tightly-coupled additions in one commit because `KeymapBuilder::with_settings` is the only consumer of `SettingsRegistry`:

- `tui_pane/src/settings.rs` — `SettingsRegistry<Ctx>` + `add_bool` / `add_enum` / `add_int` / `with_bounds` (§9).
- `tui_pane/src/keymap/builder.rs` — `KeymapBuilder<Ctx>` + `BuilderError` (§7).

**Builder hooks (post-Phase-3 review).** `KeymapBuilder` no longer takes positional `(quit, restart, dismiss)` args — framework owns those dispatches. Three optional chained hooks let the binary opt in to notification:
- `.on_quit(fn(&mut Ctx))` — fires after framework processes `GlobalAction::Quit`.
- `.on_restart(fn(&mut Ctx))` — fires after framework processes `GlobalAction::Restart`.
- `.dismiss_fallback(fn(&mut Ctx) -> bool)` — fires when framework's own dismiss chain finds nothing to dismiss; returns `true` if binary handled it.

`KeymapBuilder::build()` is where vim-mode extras are applied (per "Vim mode — framework capability" §): if `VimMode::Enabled`, append `'k'/'j'/'h'/'l'` to `Navigation::UP/DOWN/LEFT/RIGHT` (skipping any already bound on the full `KeyBind`, not just the `KeyCode`), then walk every registered `Shortcuts::vim_extras()` and append. Applied **after** TOML overlay so `[navigation]` user replacement does not disable vim.

Unit tests:
- TOML round-trip through the builder: single-key form, array form, in-array duplicate rejection.
- `BuilderError::NavigationMissing` / `GlobalsMissing` / `DuplicateScope` surface from `build()`.
- `.on_quit()` / `.on_restart()` / `.dismiss_fallback()` are reachable and stored — a unit test fires the corresponding `GlobalAction` and asserts the registered hook ran (or, for dismiss, that the fallback fires only when framework dismiss chain finds nothing).
- Vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods), not just `code`: if user binds `Shift+k` to anything, vim's `'k'` for `NavigationAction::Down` still applies (different mods). (Migrated from Phase 8 — vim application is the builder's job.)
- `MockNavigation::Up` keeps its primary as the inserted `KeyCode::Up` even with `VimMode::Enabled` applied — insertion-order primary preserved (deferred from Phase 4).

After Phase 9 the entire `tui_pane` foundation is in place: keys, action machinery, bindings, scope map, bar primitives, pane id + ctx + framework skeleton, scope traits, keymap, builder, settings registry. Framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) and the `Framework` aggregator's pane fields + helper methods land in Phase 10.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use keymap::{KeymapBuilder, BuilderError};` and `pub use settings::SettingsRegistry;`. `KeymapBuilder::build` and any internal helpers it calls into other keymap files are `pub(super)` (standing rule 3).

### Phase 10 — Framework panes

Phase 10 fills in the framework panes inside the **existing** `Framework<Ctx>` skeleton from Phase 6. The struct's pane fields and helper methods land here; the type itself, `AppContext`, and `FocusedPane` already exist.

> **Phase 6 → Phase 10 contract (mirror).** Purely additive: this phase adds fields and methods, but the Phase 6 surface (3 frozen fields: `focused`, `quit_requested`, `restart_requested`; 5 frozen methods: `new`, `focused`, `set_focused`, `quit_requested`, `restart_requested`) is **frozen verbatim**. Tests written in Phases 7–9 against the skeleton must continue to pass at the end of Phase 10. If Phase 10 implementation surfaces a better name or signature for any of the frozen items, that is a deliberate breaking change — surface it as a follow-up, not a silent rename.

Add to `tui_pane/src/panes/`:

- `keymap_pane.rs` — `KeymapPane<Ctx>` with internal `Mode::{Browse, Awaiting, Conflict}`. Method `editor_target(&self) -> Option<&Path>`.
- `settings_pane.rs` — `SettingsPane<Ctx>` with internal `Mode::{Browse, Editing}`; uses `SettingsRegistry<Ctx>`. Method `editor_target(&self) -> Option<&Path>`. `input_mode(ctx)` returns `TextInput` when `Mode == Editing`, `Navigable` otherwise.
- `toasts.rs` — `Toasts<Ctx>` stack with `ToastsAction::Dismiss` (defaults to `Esc`). The framework supplies a built-in `Shortcuts<Ctx>` impl whose dispatcher reaches the toasts stack via `AppContext::framework_mut()`. Under the post-Phase-3 design, `Toasts` also participates in the framework's `dismiss()` chain: when `GlobalAction::Dismiss` fires, framework first asks `toasts.try_pop_top()`; if nothing was on the stack, framework checks the focused framework overlay; if still nothing, falls through to the binary's `dismiss_fallback`.

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

Add `tui_pane/src/settings.rs` — `SettingsRegistry<Ctx>` + `add_bool` / `add_enum` / `add_int` builders. Each closure takes `&Ctx` / `&mut Ctx`.

**Fill in `tui_pane/src/framework/mod.rs`** — replace the Phase 6 skeleton with the real `Framework<Ctx>` aggregator owning the three framework panes plus a registry of app-pane queries:

```rust
pub struct Framework<Ctx> {
    pub keymap_pane:   KeymapPane<Ctx>,
    pub settings_pane: SettingsPane<Ctx>,
    pub toasts:        Toasts<Ctx>,
    /// Currently focused pane. Phase 6 already added this field on the
    /// skeleton; Phase 10 keeps it in place. Read via `focused()`,
    /// written via `set_focused(...)` — the binary's `Focus` subsystem
    /// is the only writer.
    focused: FocusedPane<Ctx::AppPaneId>,
    /// Lifecycle flags set by framework dispatch when `GlobalAction::Quit`
    /// / `Restart` fires. Phase 6 already added these on the skeleton;
    /// Phase 10 keeps them in place. Binary's main loop polls
    /// `quit_requested()` / `restart_requested()` every tick.
    quit_requested:    bool,
    restart_requested: bool,
    /// Per-AppPaneId queries supplied by the app at registration. Each
    /// app pane's `Shortcuts::input_mode` becomes a free fn
    /// `fn(&Ctx) -> InputMode` registered alongside its dispatcher.
    /// Framework panes are special-cased and do not appear here.
    input_mode_queries: HashMap<Ctx::AppPaneId, fn(&Ctx) -> InputMode>,
}

impl<Ctx: AppContext> Framework<Ctx> {
    pub fn focused(&self) -> FocusedPane<Ctx::AppPaneId> { self.focused }
    pub fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) { self.focused = focus; }

    pub fn editor_target_path(&self) -> Option<&Path> {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.editor_target(),
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.editor_target(),
            _ => None,
        }
    }

    pub fn focused_pane_input_mode(&self, ctx: &Ctx) -> InputMode {
        match self.focused {
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.input_mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.input_mode(ctx),
            FocusedPane::Framework(FrameworkPaneId::Toasts)   => InputMode::Static,
            FocusedPane::App(app)                              => self.input_mode_queries.get(&app)
                .map_or(InputMode::Navigable, |q| q(ctx)),
        }
    }
}
```

The registry is populated by `KeymapBuilder::register::<P>()`: each pane's impl provides a free fn `fn pane_input_mode(ctx: &App) -> InputMode` (reads pane state from `ctx`) that the registration step files into `Framework::input_mode_queries[AppPaneId::P]`.

`Framework<Ctx>` lives in `tui_pane` (skeleton from Phase 6; filled in here). The `App.framework: Framework<App>` field-add lands in **Phase 13**, when the framework panes' input paths replace the old `handle_settings_key` / `handle_keymap_key`. Before Phase 13 the filled-in framework type is exercised only by `tui_pane`'s own `cfg(test)` units and `tui_pane/tests/` integration files.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use panes::{KeymapPane, SettingsPane, Toasts};`. `panes/mod.rs` is `mod keymap_pane; mod settings_pane; mod toasts; pub use keymap_pane::KeymapPane; pub use settings_pane::SettingsPane; pub use toasts::Toasts;` (standing rule 1). New `Framework<Ctx>` getters (`editor_target_path`, `focused_pane_input_mode`, `dismiss`, etc.) get `#[must_use]` per standing rule 4 where applicable.

### Phase 11 — Framework bar renderer

Add `tui_pane/src/bar/` per the BarRegion model:

- `mod.rs` — `render(pane, ctx, keymap, framework) -> StatusBar`. Calls `pane.bar_slots(ctx)` once, walks `BarRegion::ALL`, dispatches to each region module, joins spans into `StatusBar`.
- `region.rs` — `BarRegion::{Nav, PaneAction, Global}` + `ALL` (added Phase 5).
- `slot.rs` — `BarSlot<A>`, `ShortcutState` (added Phase 5).
- `support.rs` — `format_action_keys(&[KeyBind]) -> String`, `push_cancel_row`, shared row builders.
- `nav_region.rs` — emits framework's nav + pane-cycle rows when `pane.input_mode(ctx) == Navigable`, then pane's `(Nav, _)` rows. Suppressed entirely when `input_mode == Static` or `TextInput`.
- `pane_action_region.rs` — emits pane's `(PaneAction, _)` rows. Renders for `Navigable` and `Static`; suppressed for `TextInput`.
- `global_region.rs` — emits `GlobalAction` + `AppGlobals::render_order()`; suppressed when `pane.input_mode(ctx) == TextInput`.

Depends on Phase 10 (`Framework<Ctx>` exists; framework-pane `Shortcuts<Ctx>` impls exist) plus Phase 8's `Keymap<Ctx>` lookups.

Snapshot tests in this phase cover the framework panes only (Settings Browse / Settings Editing / Keymap Browse / Keymap Awaiting / Keymap Conflict / Toasts) plus a fixture pane exercising every `BarRegion` rule. App-pane snapshots land in Phase 12 once their `Shortcuts<App>` impls exist.

**Paired-row separator policy.** Inherited from the Phase 2 retrospective decision: the `Paired` row's `debug_assert!` covers only the parser-producible `KeyCode` set; widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep. See Phase 2 review block (line 1020) for full text.

**Root re-exports (per Phase 5+ standing rule 2):** `tui_pane/src/lib.rs` adds `pub use bar::StatusBar;` (and any other public bar types not already exported in Phase 5). All `bar/` submodules declared `mod` (private) in `bar/mod.rs` per standing rule 1.

### Phase 12 — App action enums + `Shortcuts<App>` impls

**Parallel-path invariant for Phases 12–15.** The new dispatch path lands alongside the old one. The old path stays the source of truth for behavior; the new path is exercised by tests added in each phase. **Phase 16 is the only phase that deletes** old code.

**Flat-namespace paths (per Phase 5+ standing rule 2).** Every `tui_pane` import in this phase uses flat paths: `use tui_pane::KeyBind;`, `use tui_pane::GlobalAction;`, `use tui_pane::Shortcuts;`, `tui_pane::action_enum! { ... }`, `tui_pane::bindings! { ... }`. Never `tui_pane::keymap::Foo`.

**Binary-side `mod` rule (per Phase 5+ standing rule 1).** New module files added to `src/tui/` for the new action enums (e.g. `app_global_action.rs`, `navigation_action.rs`) are declared `mod foo;` at their parent (never `pub mod foo;`); facades re-export with `pub use foo::Type;`. `cargo mend` denies `pub mod` workspace-wide.

In the cargo-port binary crate:

- **`action_enum!` migration cost.** Every existing `action_enum!` invocation in `src/tui/` gains a third positional `bar_label` literal between the toml key and description, per Phase 5's grammar amendment. When the bar text matches the toml key, just duplicate the literal — no per-arm design decision. The hand-rolled `tui_pane::GlobalAction` already ships its own `bar_label` (Phase 5).
- Define `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction`.
- **Split today's `GlobalAction`** in `src/tui/keymap.rs` into `tui_pane::GlobalAction` (the framework half: Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss) and `AppGlobalAction` (binary-owned). During Phases 12–15 the binary's existing `GlobalAction` stays in place; references to the framework's enum are path-qualified as `tui_pane::GlobalAction` to disambiguate. Phase 16 deletes the binary's old enum and `use tui_pane::GlobalAction` makes the name available unqualified. (Requires `pub use keymap::GlobalAction;` at `tui_pane/src/lib.rs` crate root — add this re-export when Phase 12 lands, mirroring the Phase 3 `ActionEnum` precedent.)
- Add `ExpandRow` / `CollapseRow` to `ProjectListAction`.
- Implement `Shortcuts<App>` for each app pane (Package, Git, ProjectList, CiRuns, Lints, Targets, Output, Lang, Cpu, Finder). Each pane:
  - Owns `defaults() -> Bindings<Action>`.
  - Owns `label(&self, action, ctx) -> Option<&'static str>` and `state(&self, action, ctx) -> ShortcutState` — moves cursor-position-dependent label logic out of `App::enter_action` into the four affected impls (Package Activate label "open" for `CratesIo`, Git Activate label, Targets Activate label, CiRuns Activate hidden at EOL via `None`).
  - Registers a free dispatcher `fn(Action, &mut App)`.
  - Optionally overrides `bar_slots(ctx)` for paired layouts and data-dependent omission (ProjectList: emits `(Nav, Paired(CollapseRow, ExpandRow, "expand"))` and `(Nav, Paired(ExpandAll, CollapseAll, "all"))`; CiRuns: omits toggle row when no ci data).
  - Overrides `input_mode` for `FinderPane` → `TextInput`, `OutputPane` → `Static`. App list panes accept the default `Navigable`.
  - Overrides `vim_extras` to declare pane-action vim binds (`ProjectListAction::ExpandRow → 'l'`, `CollapseRow → 'h'`).
- Implement `Navigation<App> for AppNavigation` and `Globals<App> for AppGlobalAction`.
- Build the app's `Keymap` at startup. Old `App::enter_action` and old `for_status_bar` still exist; the new keymap is populated but not consumed yet.

**`anyhow` lands in the binary in this phase.** This is the first call site that benefits from context wrapping (`Keymap::<App>::builder(...).load_toml(path)?.build()?` → wrap with `.with_context(|| format!("loading keymap from {path:?}"))`). Add `anyhow = "1"` to the root `Cargo.toml` `[dependencies]`. The library (`tui_pane`) does not depend on `anyhow` — only typed `KeymapError` / `KeyParseError` / etc. cross the framework boundary, and the binary adds context at the boundary.

**Phase 12 tests:**
- CiRuns `pane.label(Activate, ctx)` returns `None` when the viewport cursor is at EOL (hides the slot).
- Package `pane.label(Activate, ctx)` returns `Some("open")` when on `CratesIo` field with a version, else `Some("activate")` (the default `bar_label`).
- App-pane bar snapshot tests under default bindings: one snapshot per focused-pane context (Package / Git / ProjectList / CiRuns / Lints / Targets / Output / Lang / Cpu / Finder).

### Phase 13 — Reroute overlay input handlers

Convert overlay handlers to scope dispatch:

- `handle_finder_key` (`finder.rs:567-608`) — consult `keymap.scope_for::<FinderPane>().action_for(&bind)`. Text-input fall-through stays for `Char(c)` / `Backspace` / `Delete`. Finder consults *only* `FinderAction` for navigation (`PrevMatch` / `NextMatch` / `Home` / `End`); never `NavigationAction` — this prevents vim-`'k'` leaking into the search box.
- Framework `SettingsPane`'s input path replaces today's `handle_settings_key` + `handle_settings_adjust_key` + `handle_settings_edit_key`. Browse/Editing modes route through internal mode flag.
- Framework `KeymapPane`'s input path replaces `handle_keymap_key`. Browse/Awaiting/Conflict modes route through internal mode flag.

**Phase 13 tests:**
- Rebinding `FinderAction::Cancel` to `'q'` closes finder; `'k'` typed in finder inserts `'k'` even with vim mode on.
- Binding any action to `Up` while in Awaiting capture mode produces a "reserved for navigation" rejection (replaces today's `is_navigation_reserved` semantics via scope lookup).

### Phase 14 — Reroute base-pane navigation

`KeyCode::Up`/`Down`/`Left`/`Right`/`PageUp`/`PageDown`/`Home`/`End` in `handle_normal_key` (`input.rs:580-622`), `handle_detail_key`, `handle_lints_key`, `handle_ci_runs_key` consult `NavigationAction` after the pane scope. ProjectList's `Left`/`Right` route via `ProjectListAction::CollapseRow` / `ExpandRow` (pane-scope precedence). Delete `NAVIGATION_RESERVED` (`keymap.rs:794-799`) and `is_navigation_reserved` — replaced by scope lookup against `NavigationAction`.

**Phase 14 tests:**
- Rebinding `NavigationAction::Down` to `'j'` (vim-off) moves cursor.
- Rebinding `ProjectListAction::ExpandRow` to `Tab` (with `GlobalAction::NextPane` rebound away) expands current row.

### Phase 15 — Reroute Toasts, Output, structural Esc

Convert `handle_toast_key` (`input.rs:657-684`) to consult `ToastsAction::Dismiss`. The Esc-on-output structural pre-handler at `input.rs:112-119` runs before overlays/globals/pane handlers — so pressing Esc clears `example_output` from any pane. Preserve the cross-pane semantics but route the key check through the framework:

```rust
let bind = KeyBind::from(event);
if !app.inflight.example_output().is_empty()
   && app.framework.focused_pane_input_mode(app)
       != InputMode::TextInput
   && app.keymap.scope_for::<OutputPane>().action_for(&bind) == Some(OutputAction::Cancel)
{
    let was_on_output = app.focus.is(PaneId::Output);
    app.inflight.example_output_mut().clear();
    if was_on_output { app.focus.set(PaneId::Targets); }
    return;
}
```

`focused_pane_input_mode()` returns the focused pane's `InputMode`. The check `!= InputMode::TextInput` prevents the structural Esc from firing while a Settings numeric edit is active (where Esc means "discard edit", not "clear example_output").

After Phase 15: every key dispatches through the keymap. No `KeyCode::*` direct match for command keys remains.

**Phase 15 tests:**
- Rebinding `OutputAction::Cancel` to `'q'` clears example_output from any pane.
- Rebinding `ToastsAction::Dismiss` to `'d'` dismisses focused toast via `'d'`.
- With Settings in Editing mode, pressing Esc cancels the edit instead of clearing example_output (text-input gating).

### Phase 16 — Bar swap and cleanup

Add the `What dissolves` / `What survives` summary (currently in this doc) as user-facing notes inside `tui_pane/README.md` so the published library has its own change log of what the framework absorbed.

**Binary main loop change (post-Phase-3 review).** The binary's main loop in `src/tui/terminal.rs` switches from polling `app.overlays.should_quit()` to polling `app.framework.quit_requested()` and `app.framework.restart_requested()`. The `should_quit()` accessor on `overlays` deletes; the framework owns the lifecycle flags now. If the binary needs cleanup, it registers `.on_quit(|app| { app.persist_state() })` on the builder.

Delete:

- `App::enter_action`, `shortcuts::enter()` const fn.
- The old combined `GlobalAction` enum in `src/tui/keymap.rs` (split into `tui_pane::GlobalAction` + `AppGlobalAction` in Phase 12).
- `Overlays::should_quit` accessor and the `should_quit` flag on `Overlays` — replaced by `framework.quit_requested()`.
- The seven static constants (`NAV`, `ARROWS_EXPAND`, `ARROWS_TOGGLE`, `TAB_PANE`, `ESC_CANCEL`, `ESC_CLOSE`, `EXPAND_COLLAPSE_ALL`) and all their call sites.
- `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups` per-context helpers.
- Threaded `enter_action` / `is_rust` / `clear_lint_action` / `terminal_command_configured` / `selected_project_is_deleted` parameters.
- The dead `enter_action` arm in `project_list_groups`.
- The CiRuns `Some("fetch")` label at EOL (the bar bug).
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`.

After Phase 16, `shortcuts.rs` contains only legacy types pending removal (or is deleted entirely if all callers have flipped to `Shortcuts::label` / `Shortcuts::state`). The `InputContext` enum is deleted; tests under `src/tui/app/tests/` referencing it migrate to `app.focus.current()`-based lookups in this phase.

Hoist `make_app` from `tests/mod.rs` to `src/tui/tui_test_support.rs` (`pub(super) fn make_app`); declare `#[cfg(test)] mod tui_test_support;` in `src/tui/mod.rs`.

### Phase 17 — Regression tests

Bar-on-rebind:

- Rebinding each `*Action::Activate` (`Package`, `Git`, `Targets`, `CiRuns`, `Lints`) updates that pane's bar.
- Rebinding `NavigationAction::Up` / `Down` / `Left` / `Right` updates the `↑/↓` nav row in every base-pane bar that uses it.
- Rebinding `GlobalAction::NextPane` updates the pane-cycle row.
- Rebinding `ProjectListAction::ExpandAll` / `CollapseAll` updates the `+/-` row.
- Rebinding `ProjectListAction::ExpandRow` / `CollapseRow` updates the `←/→ expand` row.
- Rebinding `FinderAction::Activate` / `Cancel` / `PrevMatch` / `NextMatch` updates the finder bar.
- Rebinding `OutputAction::Cancel` updates the output bar.
- Rebinding settings/keymap actions (framework-internal) updates their bars.

Globals + precedence:

- Globals render order matches the framework's render order then `AppGlobals::render_order()`; each slot's bar text comes from `action.bar_label()` (Phase 5's `ActionEnum::bar_label`), not a `Globals` trait method.
- CiRuns Activate at EOL renders no Enter row.
- `key_for(NavigationAction::Up) == KeyBind::from(KeyCode::Up)` even when vim mode is on.
- Rebinding `GlobalAction::Quit` to `q` keeps `q` quitting from any pane (global beats unbound).
- Rebinding `GlobalAction::NextPane` to `j` (vim-off) cycles panes from any base pane.
- Rebinding `ProjectListAction::ExpandRow` makes the pane-scope binding fire instead of `NavigationAction::Right`.
- Rebinding `FinderAction::Activate` to `Tab` while finder is open fires Activate, NOT `GlobalAction::NextPane`.

Dispatch parity (per pane, the highest-risk path):

- For each `*Action::Activate` (Package/Git/Targets/CiRuns/Lints): rebind to `'a'`, synthesize an `'a'` key event, assert the pane's free-function dispatcher ran.
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

A snapshot test per focused-pane context locks in byte-identical bar output to the pre-refactor bar under default bindings.

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
- `tui_pane` exposes (every type is at the crate root — `tui_pane::Foo` flat, never `tui_pane::keymap::Foo`): `KeyBind`, `KeyInput`, `KeyParseError`, `Bindings<A>`, `bindings!`, `ScopeMap<A>`, `Keymap<Ctx>` + `KeymapBuilder<Ctx>`, `KeymapError`, `Shortcuts<Ctx>`, `Navigation<Ctx>`, `Globals<Ctx>`, `ShortcutState`, `BarSlot<A>`, `BarRegion`, `InputMode`, `ActionEnum` + `action_enum!`, `GlobalAction`, `VimMode`, `KeymapPane<Ctx>`, `SettingsPane<Ctx>`, `Toasts<Ctx>`, `SettingsRegistry<Ctx>`, `Framework<Ctx>`, `AppContext`, `FocusedPane`, `FrameworkPaneId`. The `__bindings_arms!` helper macro is `#[doc(hidden)]` but technically reachable as `tui_pane::__bindings_arms!` (a side-effect of `#[macro_export]`); it is not part of the supported surface.
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
- All Phase 17 regression tests pass.

---

## Non-goals

- Not changing the `Pane` trait signature or any pane body's render code.
- Not unifying `PaneId::is_overlay()` semantics across the codebase — `InputContext` is being deleted, so the asymmetry resolves itself.
- Not making typed-character text input (Finder query, Settings numeric edit) keymap-driven — that's not what the keymap is for.
- Not extracting `FinderPane` into `tui_pane` in this refactor — left as a follow-up if it turns out to be reusable.
- Not migrating existing user TOML config files — old configs parse cleanly via the additive-table rule.
