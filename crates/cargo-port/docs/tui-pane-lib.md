# `tui_pane` library + universal keymap

## Design specifications

This doc is the high-level plan + roadmap. Formal specs live in sibling files:

| Spec | Covers |
|---|---|
| `phase-01-cargo-mechanics.md` | Root + library `Cargo.toml`, workspace lints (incl. `missing_docs = "deny"` from day one), resolver, `cargo install --path .` behavior, stale-path sweep, per-phase rustdoc precondition |
| `phase-01-ci-tooling.md` | CI invocation inventory, per-invocation scope decisions, auto-memory implications, recommended commit ordering |
| `phase-02-test-infra.md` | Private `test_support/` module, `cfg(test)` `TestCtx` fixture, unit-test placement, integration-test layout |
| `phase-02-core-api.md` | Full public API: `KeyBind`, `Bindings<A>`, `ScopeMap<A>`, traits, `Keymap<Ctx>`, `KeymapBuilder<Ctx>`, `Framework<Ctx>`, `SettingsRegistry<Ctx>`, errors, re-exports |
| `phase-02-macros.md` | `bindings!` and `action_enum!` formal grammar + expansion + hygiene |
| `phase-02-paneid-ctx.md` | `tui_pane::PaneId` (framework panes) / `AppPaneId` (binary) / `FocusedPane` (wrapping), `AppContext` trait, focus tracking, `App` field changes, migration order, `InputMode` query plumbing |
| `phase-02-toml-keys.md` | TOML grammar, error taxonomy, vim-after-TOML worked examples, full `KeyBind::display` / `display_short` mapping tables |

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
        │   ├── traits.rs           # Shortcuts<Ctx>, Navigation<Ctx>, Globals<Ctx>,
        │   │                       #   InputMode, ActionEnum marker, action_enum! macro
        │   ├── builder.rs          # KeymapBuilder<Ctx>; register, with_navigation,
        │   │                       #   with_globals, with_settings, vim_mode,
        │   │                       #   builder(quit, restart, dismiss)
        │   ├── global.rs           # GlobalAction enum + framework dispatch
        │   ├── vim.rs              # VimMode, vim-binding application,
        │   │                       #   vim_mode_conflicts, is_vim_reserved
        │   └── load.rs             # TOML parsing, scope replace semantics,
        │                           #   collision errors, config_path() via dirs
        ├── bar/                    # framework-owned bar renderer
        │   ├── mod.rs              # render() entry; orchestrates regions
        │   │                       #   in BarRegion::ALL order
        │   ├── region.rs           # BarRegion::{ Nav, PaneAction, Global }
        │   ├── shortcut.rs         # Shortcut + ShortcutState; BarRow<A>
        │   ├── support.rs          # format_action_keys, push_cancel_row,
        │   │                       #   shared row builders
        │   ├── nav_region.rs       # left: ↑/↓ nav, ←/→ expand, +/- all, Tab pane
        │   ├── pane_action_region.rs # center: per-action rows from focused
        │   │                       #   pane's bar_rows + shortcut
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

For the rest of this doc, signatures use `Ctx` (or `Ctx: AppContext`) when referring to the app context. See `phase-02-paneid-ctx.md` §2 for the full trait.

### Pane id design

Two enums + a wrapping type:

- `tui_pane::FrameworkPaneId { Keymap, Settings, Toasts }` — the framework's three internal panes. Always written `FrameworkPaneId` (no inside-the-crate `PaneId` short form) so the type name is one-to-one across binary and library.
- `cargo_port::AppPaneId { Package, Git, ProjectList, … }` — cargo-port's 10 panes. Hand-written enum in `src/tui/panes/spec.rs` (today's enum, minus the framework variants).
- `tui_pane::FocusedPane<AppPaneId> { App(AppPaneId), Framework(FrameworkPaneId) }` — generic wrapper used in framework trait signatures. The binary uses this directly for focus tracking.

Linking the runtime tag to the compile-time pane type: every `Shortcuts<App>` impl declares `const APP_PANE_ID: AppPaneId`. Calling `register::<PackagePane>()` records that value alongside the pane's dispatcher — registration populates the runtime mapping. The `AppPaneId` enum is the runtime side of the same registration.

Cargo-port's existing `tui::panes::PaneId` enum becomes a type alias `pub type PaneId = tui_pane::FocusedPane<AppPaneId>;` so existing call sites that name `PaneId` keep compiling; only the framework variants move out of the enum body.

See `phase-02-paneid-ctx.md` §1 for full type definitions and call-site rewrites.

### `Shortcuts` — pane scopes (state-bearing)

```rust
pub trait Shortcuts<Ctx: AppContext>: 'static {
    type Action: ActionEnum + 'static;
    const SCOPE_NAME: &'static str;

    fn defaults() -> Bindings<Self::Action>;
    fn shortcut(&self, action: Self::Action, ctx: &Ctx) -> Option<Shortcut>;

    /// Bar render rows. Owned `Vec`; cheap (N ≤ 10) and ratatui's
    /// per-frame work dwarfs the allocation. Each row carries the
    /// `BarRegion` it lands in; most panes return
    /// `(BarRegion::PaneAction, Single(action))` for every action, but
    /// ProjectList additionally returns `(BarRegion::Nav, Paired(…))`
    /// for its expand/collapse pairs. Default impl returns one
    /// `(PaneAction, Single(action))` per `Action::ALL` in declaration
    /// order; override to introduce paired rows, route into `Nav`, or
    /// to omit data-dependent rows.
    fn bar_rows(&self, ctx: &Ctx) -> Vec<(BarRegion, BarRow<Self::Action>)> {
        Self::Action::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarRow::Single(a)))
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
    fn input_mode(&self) -> InputMode { InputMode::Navigable }

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
- `shortcut(action, ctx) -> Option<Shortcut>` — returns the bar entry for an action. `None` = hidden. Cursor-position-dependent labels (CiRuns Activate at EOL hidden, Package Activate label = "open" when on `CratesIo` field) live here, not in `App::enter_action`.
- `bar_rows(ctx)` — declares the row layout per-frame. Most panes accept the default (one row per action, declaration order). Panes with paired rows override to return `vec![BarRow::Paired(NavUp, NavDown, "nav"), BarRow::Single(Activate), …]`. Data-dependent omission (CiRuns toggle row only when ci data is present) lives here too.
- `input_mode` — `InputMode::Navigable` / `Static` / `TextInput`. Gates Nav region (only when `Navigable`), Global strip (suppressed on `TextInput`), and structural Esc pre-handler (suppressed on `TextInput`).
- `vim_extras` — pane-action vim bindings (separate from `Navigation`'s arrow → vim mapping).
- `dispatcher` — returns a free function pointer. Framework calls `dispatcher()(action, ctx)`.

### `BarRow` enum

```rust
pub enum BarRow<A> {
    Single(A),                  // one action, full key list shown via display_short joined by ','
    Paired(A, A, &'static str), // two actions glued with `/`, one shared label, primary keys only
}
```

Framework rendering:
- `Single(action)` → renders all keys bound to `action` (joined by `,` after `display_short`) `<space>` `shortcut.label`.
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
    fn bar_label(action: Self::Action) -> &'static str;
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}
```

The app's *additional* globals scope (Find, OpenEditor, Rescan, etc.). The framework's pane-management/lifecycle globals are owned separately by `GlobalAction` (below); the app does not redefine them.

---

## `Keymap<Ctx>` runtime container

> Formal API in `phase-02-core-api.md` §6 (`Keymap<Ctx>`) and §7 (`KeymapBuilder<Ctx>`, including the type-state choice, `BuilderError` variants, required vs optional methods).

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

## `Shortcut` (returned by `Shortcuts::shortcut`)

```rust
pub enum ShortcutState { Enabled, Disabled }

pub struct Shortcut {
    pub label: &'static str,
    pub state: ShortcutState,
}

impl Shortcut {
    pub fn enabled(label: &'static str) -> Self { … }
    pub fn disabled(label: &'static str) -> Self { … }
}
```

`shortcut(action, ctx) -> Option<Shortcut>`: `None` = hidden; `Some(Shortcut::enabled(label))` = visible & enabled; `Some(Shortcut::disabled(label))` = visible & grayed out.

The framework adds the bound key (looked up via `display_keys_for(action)`) when rendering. The pane never builds a key string.

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

> Formal API in `phase-02-core-api.md` §2; macro grammar + expansion in `phase-02-macros.md` §1.

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

> Formal API in `phase-02-core-api.md` §3.

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

When a single pane has multiple modes (Browse vs Editing for Settings, Browse vs Awaiting vs Conflict for Keymap), use an internal mode flag and route via `shortcut()` and `dispatch()`. Do **not** create a separate `*Pane` type per mode.

Mode-neutral action names (`Activate`, `Cancel`, `Left`, `Right`) describe the user's intent; the pane decides what each intent does in each mode:

```rust
impl Shortcuts<App> for SomePane {
    fn shortcut(&self, action: SomeAction, ctx: &App) -> Option<Shortcut> {
        match (self.mode, action) {
            (Mode::Browse, SomeAction::Activate) => Some(Shortcut::enabled("edit")),
            (Mode::Edit,   SomeAction::Activate) => Some(Shortcut::enabled("confirm")),
            // …
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

> Formal API in `phase-02-core-api.md` §9 (closure signatures, persistence semantics).

Each setting carries: TOML key, display label, value-getter closure, value-setter closure. Three value flavors covered: `bool`, `enum` (closed string set), `int` (with optional min/max).

The app provides only data + closures. It writes no `Shortcuts` impl, no mode state machine, no overlay rendering.

---

## `GlobalAction` — framework base, app extension

> Formal enum + dispatch hooks in `phase-02-core-api.md` §10. TOML grammar for the merged `[global]` table in `phase-02-toml-keys.md` §1–2.

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

Framework owns defaults (`q` → Quit, `R` → Restart, `Tab` → NextPane, `Shift+Tab` → PrevPane, `Ctrl+K` → OpenKeymap, `s` → OpenSettings, `x` → Dismiss), the bar entries, and most dispatch.

### App-injected dispatch hooks

`Quit`, `Restart`, `Dismiss` need to reach app state (`app.dismiss(focused_dismiss_target())`, the quit/restart flags). All three are **required**: the builder takes them as positional arguments to `Keymap::builder(quit, restart, dismiss)` so omission is a compile error rather than a panic at dispatch.

```rust
fn quit(app: &mut App) { app.set_quit() }
fn restart(app: &mut App) { app.set_restart() }
fn dismiss(app: &mut App) {
    let target = app.focused_dismiss_target();
    app.dismiss(target);
}

Keymap::<App>::builder(quit, restart, dismiss)
    .vim_mode(VimMode::Enabled)
    .register::<PackagePane>()
    // …
```

The two-step bind in `dismiss` resolves the borrow conflict that `|app| app.dismiss(app.focused_dismiss_target())` would otherwise hit.

`NextPane` / `PrevPane` / `OpenKeymap` / `OpenSettings` are pure pane-focus operations the framework owns directly (it knows the registered pane set).

### App globals

App declares its own additional globals:

```rust
// cargo-port
pub enum AppGlobalAction { Rescan, OpenEditor, OpenTerminal, Find }
impl Globals<App> for AppGlobalAction { … }
```

Both `GlobalAction` and `AppGlobalAction` share a single TOML table named `[global]`. The loader matches each TOML key against both enums; whichever variant accepts the key is the action that gets bound. From the user's perspective there's one globals namespace — they write `[global] quit = "q"` (framework variant) and `[global] find = "/"` (app variant) into the same table.

Bar's right-hand strip: framework renders `GlobalAction` items first, then `AppGlobals::render_order()`.

### Per-action revert policy for `[global]`

The `[global]` scope uses a more permissive error policy than other scopes (which fully replace on present, defaults on absent). Per-action behavior:

- Each TOML entry in `[global]` is processed independently.
- If the value parses cleanly and doesn't collide with another binding in `[global]`, apply it.
- If the value fails to parse OR collides → emit a warning, revert *just that action* to its default, continue processing the rest of the table.
- Required actions (`Quit`, `Restart`, `Dismiss`) that the user accidentally drops or invalidates are restored to their defaults at the end of the pass.

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

> Full grammar, error taxonomy, vim-after-TOML worked examples, and `KeyBind::display` / `display_short` mapping tables in `phase-02-toml-keys.md`.

Framework handles all TOML loading. Each registered scope's `SCOPE_NAME` constant drives table lookup; framework parses every recognized table, replaces that scope's bindings, leaves missing tables at their declared defaults+vim. App provides no TOML hooks.

### TOML errors

- In-array duplicates: `key = ["Enter", "Enter"]` → parse error.
- Cross-action duplicates within a non-globals scope (e.g. `[finder] activate = "Enter"` and `cancel = "Enter"`) → parse error (return `Err`).
- The `[global]` scope follows the per-action revert policy described under `GlobalAction` above — broken individual bindings revert to defaults; the loader returns `Ok` with a list of warnings.

The `ScopeMap::insert` `debug_assert` catches the same conditions for `defaults()` builders; the TOML loader returns them as real errors or warnings.

See `phase-02-toml-keys.md` for the full grammar, error taxonomy, and worked vim-after-TOML examples.

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

| Pane | `input_mode` | `bar_rows` |
|---|---|---|
| `KeymapPane` (Browse) | `Navigable` | `[(PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `KeymapPane` (Awaiting) | `TextInput` | `[(PaneAction, Single(Cancel))]` (user is capturing a keystroke) |
| `KeymapPane` (Conflict) | `Static` | `[(PaneAction, Single(Clear)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Browse) | `Navigable` | `[(Nav, Paired(ToggleBack, ToggleNext, "toggle")), (PaneAction, Single(Activate)), (PaneAction, Single(Cancel))]` |
| `SettingsPane` (Editing) | `TextInput` | `[(PaneAction, Single(Confirm)), (PaneAction, Single(Cancel))]` |
| `Toasts` | `Static` | `[(PaneAction, Single(Dismiss))]` |

### Trait change

`Shortcuts::bar_rows` returns `(region, row)` pairs:

```rust
fn bar_rows(&self, ctx: &Ctx) -> Vec<(BarRegion, BarRow<Self::Action>)> {
    Self::Action::ALL.iter()
        .copied()
        .map(|a| (BarRegion::PaneAction, BarRow::Single(a)))
        .collect()
}
```

The default `(PaneAction, Single(action))` covers the common case. ProjectList overrides to additionally emit:

```rust
(BarRegion::Nav, BarRow::Paired(CollapseRow, ExpandRow, "expand")),
(BarRegion::Nav, BarRow::Paired(ExpandAll,   CollapseAll, "all")),
```

### Render orchestration

`bar/mod.rs::render()` calls `pane.bar_rows(ctx)` once, reads `pane.input_mode()`, then walks `BarRegion::ALL` and dispatches:

- `BarRegion::Nav` → `nav_region::render(pane, ctx, keymap, &rows)` — emits framework's nav + pane-cycle rows plus any `(Nav, _)` rows from `rows`. Skipped unless `input_mode == Navigable`.
- `BarRegion::PaneAction` → `pane_action_region::render(pane, ctx, keymap, &rows)` — emits every `(PaneAction, _)` row, calling `pane.shortcut(action, ctx)` for the label/state.
- `BarRegion::Global` → `global_region::render(keymap, framework)` — emits `GlobalAction` + `AppGlobals::render_order()`. Skipped when `input_mode == TextInput`.

Each region module returns `Vec<Span>`; `mod.rs` joins them left-to-right with framework-owned spacing into a single `StatusBar`.

## Bar architecture — framework-owned

The status bar is a framework feature. App authors write no bar layout code. See the `BarRegion` section above for the three-region model and the `bar/` module structure.

| Concern | Owner |
|---|---|
| Region orchestration | Framework — `bar/mod.rs` walks `BarRegion::ALL` |
| `Nav` region (paired rows from `Navigation` + pane-cycle from `GlobalAction::NextPane`) | Framework — `bar/nav_region.rs`; emitted only when `pane.input_mode() == Navigable` |
| `PaneAction` region | Framework — `bar/pane_action_region.rs`; emits `(PaneAction, _)` rows from `pane.bar_rows(ctx)`, calling `pane.shortcut(action, ctx)` for label/state |
| `Global` region (`GlobalAction` + `AppGlobals::render_order()`) | Framework — `bar/global_region.rs`; suppressed when `pane.input_mode() == TextInput` |
| Color / style / spacing | Framework |
| Per-action label & enabled state | Pane (via `Shortcuts::shortcut`) |
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

Pane is the focused one. Framework calls `pane.bar_rows(ctx)` once, then walks `BarRegion::ALL`: each region module filters for its own region tag and emits spans. Region rendering consults `pane.input_mode()` for suppression (`Nav` skipped unless `Navigable`; `Global` skipped on `TextInput`). Result is a single `StatusBar` value the binary draws to the frame.

**Monomorphization boundary:** `render_status_bar` is monomorphized per pane type at the binary's match-on-`focus.current()` site (see "Bar render — concrete dispatch" below). Each instantiation produces a `StatusBar`. The framework never holds a heterogeneous `Vec<BarRow<dyn Action>>`; pane types are concrete at the call site.

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

Each app pane owns its own `*Action` enum, declared next to the pane. The `action_enum!` macro (re-exported from `tui_pane`) enforces the TOML-key + description + `ALL` slice contract.

```rust
// tui_pane re-export
pub use tui_pane::action_enum;

// src/tui/panes/package.rs
action_enum! {
    pub enum PackageAction {
        Activate => "activate", "Open / activate selected field";
        Clean    => "clean",    "Clean target dir";
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
        Up       => "up",        "Move cursor up";
        Down     => "down",      "Move cursor down";
        Left     => "left",      "Move cursor left / collapse";
        Right    => "right",     "Move cursor right / expand";
        PageUp   => "page_up",   "Page up";
        PageDown => "page_down", "Page down";
        Home     => "home",      "Jump to top";
        End      => "end",       "Jump to bottom";
    }
}

action_enum! {
    pub enum FinderAction {
        Activate  => "activate",   "Go to selected match";
        Cancel    => "cancel",     "Close finder";
        PrevMatch => "prev_match", "Previous match";
        NextMatch => "next_match", "Next match";
        Home      => "home",       "Jump to first match";
        End       => "end",        "Jump to last match";
    }
}

action_enum! {
    pub enum OutputAction {
        Cancel => "cancel", "Close output";
    }
}

action_enum! {
    pub enum AppGlobalAction {
        Rescan       => "rescan",        "Rescan projects";
        OpenEditor   => "open_editor",   "Open editor for selected project";
        OpenTerminal => "open_terminal", "Open terminal";
        Find         => "find",          "Open finder";
    }
}
```

`ProjectListAction` (existing variants: `ExpandAll`, `CollapseAll`, `Clean`) gains:

```rust
ExpandRow   => "expand_row",   "Expand current node";   // today: KeyCode::Right / 'l'
CollapseRow => "collapse_row", "Collapse current node"; // today: KeyCode::Left  / 'h'
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

`render.rs:531-558` today calls `app.input_context()`-driven `for_status_bar`. Post-deletion, the framework call dispatches off `app.focus.current()` (split between `PaneId::App(_)` and `PaneId::Framework(_)` per the wrapper enum). See `phase-02-paneid-ctx.md` §1 for the canonical dispatch site — the framework's three panes are routed through a single `bar::render_framework(id, ...)` arm rather than enumerated inline.

The `Settings` / `Keymap` / `Toasts` panes use their internal mode flags (Browse/Editing, Browse/Awaiting/Conflict, etc.) to vary `bar_rows` and `shortcut` output. The current `InputContext::SettingsEditing` / `KeymapAwaiting` / `KeymapConflict` arms collapse into pane-internal mode dispatch.

`overlay_editor_target_path` (`input.rs:413`) becomes `app.framework.editor_target_path()` — Settings and Keymap panes each expose `fn editor_target(&self) -> Option<&Path>`; framework chooses based on which is focused.

---

## Phases

Each phase is a single mergeable commit. Each commit must build green and pass `cargo nextest run`. No sub-phases (`Na/Nb/Nc`) — every increment gets its own integer.

### Phase 1 — Workspace conversion ✅

Convert `cargo-port-api-fix` into a Cargo workspace. See `phase-01-cargo-mechanics.md` for full TOML and `phase-01-ci-tooling.md` for the CI updates.

Concrete steps:

1. Root `Cargo.toml` keeps `[package]` (binary) and adds `[workspace] members = ["tui_pane"]` + `resolver = "3"` (resolver must be explicit; not inferred from edition 2024 in workspace context).
2. Promote the existing `[lints.clippy]` and `[lints.rust]` blocks verbatim to `[workspace.lints.clippy]` / `[workspace.lints.rust]` (including `missing_docs = "deny"` from day one). Root `[lints]` becomes `workspace = true`.
3. Create `tui_pane/` as a sibling directory (not `crates/tui_pane/`) with `Cargo.toml` (`crossterm`, `ratatui`, `dirs` deps; `[lints] workspace = true`) and `src/lib.rs` carrying crate-level rustdoc.
4. Add `tui_pane = { path = "tui_pane", version = "0.0.4-dev" }` to the binary's `[dependencies]`.
5. Apply the CI flag updates flagged in `phase-01-ci-tooling.md` §2 (e.g. `cargo +nightly fmt --all`, `cargo mend --workspace --all-targets`, `cargo check --workspace` in the post-tool-use hook). Per the spec these can ship in a separate prior commit since they're no-ops on the current single-crate layout.
6. Update auto-memory `feedback_cargo_nextest.md` to clarify default `cargo nextest run` only tests the root package; iteration loops should pass `-p` or `--workspace`. `feedback_cargo_install.md` is unchanged (the binary stays at root).

After Phase 1: `cargo build` from the root builds both crates; `cargo install --path .` still installs the binary; `Cargo.lock` and `target/` stay at the workspace root.

**Per-phase rustdoc precondition.** Phases 2–17 add `pub` items to `tui_pane`. Each pub item ships with a rustdoc summary line — `missing_docs = "deny"` is workspace-wide from Phase 1, so a missing doc breaks the build.

### Phases 2–9 — `tui_pane` foundations

Phases 2–9 land the entire `tui_pane` public surface in dependency order, one mergeable commit per phase. The canonical type spec is `phase-02-core-api.md` (sections referenced from each sub-phase below); §11 is the canonical module hierarchy and the public re-export set in `lib.rs`. Type detail per file is in `phase-02-core-api.md` §§1–10.

**Strictly additive across Phases 2–9.** Nothing moves out of the binary in this group. The binary continues to use its in-tree `keymap_state::Keymap`, `shortcuts::*`, etc., untouched. The migration starts in Phase 12.

**Pre-Phase-2 precondition (post-tool-use hook).** Decide hook strategy before Phase 2 lands: repo-local override at `.claude/scripts/hooks/post-tool-use-cargo-check.sh` adding `--workspace`, vs. updating the global script at `~/.claude/scripts/hooks/post-tool-use-cargo-check.sh`. Without the flag, edits to `tui_pane/src/*.rs` from inside the binary working dir will not surface `tui_pane` errors. Repo-local override is the lower-blast-radius option.

**README precondition (Phase 9).** `tui_pane/README.md` lands at the end of Phase 9 — when the public API is complete. It covers crate purpose + a minimal example using `Framework::new(initial_focus)`. Code blocks in the README are ` ```ignore ` (no doctests in this crate).

### Phase 2 — Keys ✅

Add `tui_pane/src/keymap/key_bind.rs` (`KeyBind`, `KeyInput`, `KeyParseError`) per `phase-02-core-api.md` §1. Leaf types — nothing else in `tui_pane` depends on them yet.

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
- `KeyBind::shift` / `ctrl` were respec'd to take `impl Into<Self>` (i.e. `impl Into<KeyBind>`) rather than `impl Into<KeyCode>`. Reason: crossterm's `KeyCode` does not implement `From<char>`, so the planned `impl Into<KeyCode>` bound rejects `KeyBind::shift('g')`. Taking `Into<KeyBind>` reuses the three `From` impls and makes `shift`/`ctrl` composable (`KeyBind::ctrl(KeyBind::shift('g'))` → CTRL|SHIFT). `phase-02-core-api.md` §1 still lists the old signature.
- `KeyParseError` ships with 3 variants (`Empty`, `UnknownKey`, `UnknownModifier`) — `InvalidChar` from the spec was dropped because no parser path emits it. `phase-02-core-api.md` §1 still lists 4 variants.
- Parser supports `"Control"` as a synonym for `"Ctrl"` (both produce `KeyModifiers::CONTROL`); `"Space"` parses to `KeyCode::Char(' ')`. Neither was called out in the plan.

**Surprises:**
- `KeyCode` has no `From<char>` impl in crossterm — and orphan rules block adding one. This forced the `impl Into<Self>` rework.
- Modifier display order (`Ctrl` → `Alt` → `Shift`) and the case-preservation policy in `parse` (`"Ctrl+K"` → `Char('K')`, not `Char('k')`) are now baked into Phase 2 tests. Phase 8 (TOML loader) inherits both as facts; if the loader needs case-insensitive letter lookup, that is a *keymap-layer* normalization, not a `KeyBind::parse` concern.

**Implications for remaining phases:**
- `phase-02-core-api.md` §1 is out of sync with shipped code (signatures + error variants). Update before any later phase reads it as canonical.
- Phase 8 (`Keymap<Ctx>` + TOML loader) must decide letter-case normalization policy explicitly — `parse` preserves case as-is.
- Future framework error types (`KeymapError` Phase 4 skeleton, fill in Phase 8) should use `#[derive(thiserror::Error)]` with `#[from] KeyParseError` for source chaining, per the pattern established here.

#### Phase 2 Review

- Phase 3: rename `keymap/traits.rs` → `keymap/action_enum.rs` so the file name matches its sole resident (`ActionEnum` + `action_enum!`) and does not collide with Phase 7's per-trait file split.
- Phase 4: `KeymapError` ships with `#[derive(thiserror::Error)]` + `#[from] KeyParseError` for source chaining, and unit tests are rescoped to constructs that exist by end of Phase 4 (vim-application test deferred to Phase 9). `bindings!` macro tests now cover composed `KeyBind::ctrl(KeyBind::shift('g'))`.
- Phase 8: loader explicitly lowercases single-letter TOML keys (so `quit = "Q"` binds `Char('q')`); modifier display order is canonical `Ctrl+Alt+Shift+key` (no round-trip ordering preservation); vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods); `KeymapError` source chain from `KeyParseError` is asserted.
- Phase 11: paired-row separator policy made explicit — `Paired::debug_assert!` covers only the parser-producible `KeyCode` set; exotic variants may panic, and widening the bindable set requires widening Phase 2's `display_short_no_separators` test in lockstep.
- Phase 12: `anyhow = "1"` lands in the binary's `Cargo.toml` here (first call site that needs context wrapping is `Keymap::<App>::builder(...).load_toml(?)?.build()?`).
- §1 (`Pane id design`): `PaneId` → `FrameworkPaneId` everywhere, including the inside-the-crate short form, so the type name is one-to-one across library and binary call sites.
- `phase-02-core-api.md` §1 + `tui-pane-lib.md` §11 lib.rs sketch synced to shipped Phase 2 code: `shift`/`ctrl` take `impl Into<Self>`, `From<KeyEvent>` documented, 3-variant `KeyParseError` (`InvalidChar` dropped), parser policy (`"Control"` synonym, `"Space"` token, case-preserving) called out.
- `phase-02-toml-keys.md` synced to the Zed/VSCode/Helix-aligned letter-case decision: loader lowercases single-letter ASCII keys (`"Q"` → `Char('q')`, never `Shift+q`); modifier tokens are case-insensitive on input but writeback canonical capitalized; named-key tokens (`Enter`, `Tab`, `F1`, …) are case-sensitive with no aliases; non-ASCII letters not lowercased; modifier repeats silently OR'd (not rejected — bitwise OR is idempotent).
- Phase 6 + Phase 10 now spell out the **Phase 6 → Phase 10 contract**: Phase 6 freezes a 1-field / 3-method `Framework<Ctx>` skeleton (`focused` field, `new`/`focused()`/`set_focused()`); Phase 10 is purely additive on top. Mirrored at both phase blocks so neither side can drift independently.
- Decided: `KeyEvent` press/release/repeat handling uses a typed wrapper enum (`KeyInput { Press, Release, Repeat }`) at the framework boundary, not a runtime check at each dispatch site and not a fallible `Option`-returning conversion. Modeled after Zed/GPUI's typed-event split. Repeat is preserved (not collapsed into Press) so future handlers can opt into auto-repeat behavior. Phases 13–15 dispatch sites pattern-match `KeyInput::Press(bind)` (or call `.press()`); the event-loop entry produces `KeyInput` once.

### Phase 3 — Action machinery

Add `tui_pane/src/keymap/action_enum.rs` with `ActionEnum` + `action_enum!` (per §4 — the trait part; the three scope traits land in Phase 7). Add `tui_pane/src/keymap/base_globals.rs` with `GlobalAction` and its `ActionEnum` impl (§10). Add `tui_pane/src/keymap/vim.rs` with `VimMode::{Disabled, Enabled}` (§10).

> File renamed from `traits.rs` to `action_enum.rs` so its name matches its contents — the three scope traits live in their own files (`shortcuts.rs` / `navigation.rs` / `globals.rs`) per Phase 7. A file called `traits.rs` holding just one marker trait would mislead.

### Phase 4 — Bindings, scope map, loader errors

Add `tui_pane/src/keymap/bindings.rs` (`Bindings<A>` + `bindings!`, §2), `tui_pane/src/keymap/scope_map.rs` (`ScopeMap<A>`, §3), and `tui_pane/src/keymap/load.rs` skeleton holding `KeymapError` (§10). The loader's actual TOML-parsing impl lands in Phase 8 alongside `Keymap<Ctx>`.

`KeymapError` is `#[derive(thiserror::Error)]` and includes `#[from] KeyParseError` on whichever variant wraps a parse failure (e.g. `InvalidBinding { line: usize, #[from] source: KeyParseError }`). This gives Phase 8's TOML loader free `?` propagation from `KeyBind::parse` and free `Display` chains via `thiserror`'s source linking.

`bindings!` macro grammar must accept arbitrary `impl Into<KeyBind>` expressions on the RHS — including composed forms like `KeyBind::ctrl(KeyBind::shift('g'))` (CTRL|SHIFT, established by Phase 2). The macro's unit tests cover the composed case.

Unit tests (this phase, scoped to what exists by end of Phase 4):
- `Bindings::insert` preserves insertion order; first key for an action is the primary.
- `ScopeMap::add_bindings` on an empty map produces `by_key.len() == by_action.values().map(Vec::len).sum::<usize>()` (no orphan entries).
- `bindings!` accepts `KeyBind::ctrl(KeyBind::shift('g'))` and stores `KeyModifiers::CONTROL | SHIFT`.
- (Deferred to Phase 9, when the builder + `VimMode::Enabled` application pipeline exist:) `MockNavigation::Up` keeps its primary as the inserted `KeyCode::Up` even with `VimMode::Enabled` applied — insertion-order primary.

### Phase 5 — Bar primitives

Add `tui_pane/src/bar/region.rs` (`BarRegion::{Nav, PaneAction, Global}` + `ALL`), `tui_pane/src/bar/shortcut.rs` (`Shortcut`, `ShortcutState`, `BarRow<A>`), and `InputMode` in `bar/mod.rs`. All per §5.

Leaf types. First consumed by the scope traits in Phase 7.

### Phase 6 — Pane identity, ctx, Framework skeleton

The chicken-and-egg unit. `AppContext::framework()` returns `&Framework<Self>` and `Framework<Ctx>` requires `Ctx: AppContext`, so they must land together. `AppContext::set_focus` takes `FocusedPane<Self::AppPaneId>`, so the pane-id types come along.

Add:

- `tui_pane/src/pane_id.rs` — `FrameworkPaneId::{Keymap, Settings, Toasts}`, `FocusedPane<AppPaneId>::{App, Framework}`.
- `tui_pane/src/app_context.rs` — `AppContext` trait (`type AppPaneId`, `framework`, `framework_mut`, `set_focus`).
- `tui_pane/src/framework/mod.rs` — `Framework<Ctx>` **skeleton** (one field, three methods, frozen):

```rust
pub struct Framework<Ctx: AppContext> {
    focused: FocusedPane<Ctx::AppPaneId>,
}

impl<Ctx: AppContext> Framework<Ctx> {
    pub fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self { ... }
    pub fn focused(&self) -> &FocusedPane<Ctx::AppPaneId>     { ... }
    pub fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) { ... }
}
```

> **Phase 6 → Phase 10 contract.** This 1-field / 3-method API is **frozen at Phase 6 and must survive Phase 10 verbatim.** Phase 10 is purely additive: it adds the `keymap_pane` / `settings_pane` / `toasts` fields, the `input_mode_queries` / `editor_target_path` / `focused_pane_input_mode` plumbing, and any new query methods — but it **never renames** `focused`, `new`, `focused()`, or `set_focused()`. Tests written in Phases 7–9 against this surface stay green when Phase 10 lands. The cost: Phase 6 has to commit to method names that still read right after Phase 10 fills the struct out — if Phase 10 wishes one were called something else, too late.

No pane fields, no `input_mode_queries`, no `editor_target_path`, no `focused_pane_input_mode` in Phase 6 — those land in Phase 10 once framework panes exist.

### Phase 7 — Scope traits

Split §4 into one file per trait (each is independent, the heaviest is `Shortcuts<Ctx>` with 10+ items):

- `tui_pane/src/keymap/shortcuts.rs` — `Shortcuts<Ctx>`.
- `tui_pane/src/keymap/navigation.rs` — `Navigation<Ctx>`.
- `tui_pane/src/keymap/globals.rs` — `Globals<Ctx>` (app-extension globals, separate from the framework's own `GlobalAction` from Phase 3).

`keymap/action_enum.rs` (added in Phase 3) keeps `ActionEnum` + `action_enum!` only.

### Phase 8 — Keymap container

Add `tui_pane/src/keymap/mod_.rs` with `Keymap<Ctx>` + `scope_for` / `navigation` / `globals` / `base_globals` / `config_path` (per §6). Fill in the actual TOML-parsing implementation in `keymap/load.rs` (skeleton + `KeymapError` from Phase 4). Construction is via `Keymap::builder(quit, restart, dismiss)` — the builder itself lands in Phase 9.

**Loader-layer decisions established here (Zed/VSCode/Helix-aligned):**
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
- Vim-mode skip-already-bound is keyed on full `KeyBind` equality (code + mods), not just `code`: if user binds `Shift+k` to anything, vim's `'k'` for `NavigationAction::Down` still applies (different mods).
- `KeyParseError` from `KeyBind::parse` chains into `KeymapError` via `#[from]` — round-trip a malformed binding string and assert the source error is preserved (`err.source().is_some()`).

### Phase 9 — Keymap builder + settings registry

Two tightly-coupled additions in one commit because `KeymapBuilder::with_settings` is the only consumer of `SettingsRegistry`:

- `tui_pane/src/settings.rs` — `SettingsRegistry<Ctx>` + `add_bool` / `add_enum` / `add_int` / `with_bounds` (§9).
- `tui_pane/src/keymap/builder.rs` — `KeymapBuilder<Ctx>` + `BuilderError` (§7).

Unit tests:
- TOML round-trip through the builder: single-key form, array form, in-array duplicate rejection.
- `BuilderError::NavigationMissing` / `GlobalsMissing` / `DuplicateScope` surface from `build()`.

After Phase 9 the entire `tui_pane` foundation is in place: keys, action machinery, bindings, scope map, bar primitives, pane id + ctx + framework skeleton, scope traits, keymap, builder, settings registry. Framework panes (`KeymapPane`, `SettingsPane`, `Toasts`) and the `Framework` aggregator's pane fields + helper methods land in Phase 10.

### Phase 10 — Framework panes

Phase 10 fills in the framework panes inside the **existing** `Framework<Ctx>` skeleton from Phase 6. The struct's pane fields and helper methods land here; the type itself, `AppContext`, and `FocusedPane` already exist.

> **Phase 6 → Phase 10 contract (mirror).** Purely additive: this phase adds fields and methods, but the Phase 6 surface (`focused` field, `new(initial_focus)`, `focused()`, `set_focused(...)`) is **frozen verbatim**. Tests written in Phases 7–9 against the skeleton must continue to pass at the end of Phase 10. If Phase 10 implementation surfaces a better name or signature for any of those four, that is a deliberate breaking change — surface it as a follow-up, not a silent rename.

Add to `tui_pane/src/panes/`:

- `keymap_pane.rs` — `KeymapPane<Ctx>` with internal `Mode::{Browse, Awaiting, Conflict}`. Method `editor_target(&self) -> Option<&Path>`.
- `settings_pane.rs` — `SettingsPane<Ctx>` with internal `Mode::{Browse, Editing}`; uses `SettingsRegistry<Ctx>`. Method `editor_target(&self) -> Option<&Path>`. `input_mode()` returns `TextInput` when `Mode == Editing`, `Navigable` otherwise.
- `toasts.rs` — `Toasts<Ctx>` stack with `ToastsAction::Dismiss` (defaults to `Esc`). The framework supplies a built-in `Shortcuts<Ctx>` impl whose dispatcher needs to reach the toasts stack via `AppContext::framework_mut()`. Toasts' dispatcher is a free fn `fn dismiss_toast<Ctx: AppContext>(_: ToastsAction, ctx: &mut Ctx)` that calls `ctx.framework_mut()`.

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
            FocusedPane::Framework(FrameworkPaneId::Settings) => self.settings_pane.input_mode(),
            FocusedPane::Framework(FrameworkPaneId::Keymap)   => self.keymap_pane.input_mode(),
            FocusedPane::Framework(FrameworkPaneId::Toasts)   => InputMode::Static,
            FocusedPane::App(app)                              => self.input_mode_queries.get(&app)
                .map_or(InputMode::Navigable, |q| q(ctx)),
        }
    }
}
```

The registry is populated by `KeymapBuilder::register::<P>()`: each pane's impl provides a free fn `fn pane_input_mode(ctx: &App) -> InputMode` (reads pane state from `ctx`) that the registration step files into `Framework::input_mode_queries[AppPaneId::P]`.

`Framework<Ctx>` lives in `tui_pane` (skeleton from Phase 6; filled in here). The `App.framework: Framework<App>` field-add lands in **Phase 13**, when the framework panes' input paths replace the old `handle_settings_key` / `handle_keymap_key`. Before Phase 13 the filled-in framework type is exercised only by `tui_pane`'s own `cfg(test)` units and `tui_pane/tests/` integration files.

### Phase 11 — Framework bar renderer

Add `tui_pane/src/bar/` per the BarRegion model:

- `mod.rs` — `render(pane, ctx, keymap, framework) -> StatusBar`. Calls `pane.bar_rows(ctx, &mut sink)` once, walks `BarRegion::ALL`, dispatches to each region module, joins spans into `StatusBar`.
- `region.rs` — `BarRegion::{Nav, PaneAction, Global}` + `ALL`.
- `shortcut.rs` — `Shortcut`, `ShortcutState`, `BarRow<A>`, `BarRowSink<A>`.
- `support.rs` — `format_action_keys(&[KeyBind]) -> String`, `push_cancel_row`, shared row builders.
- `nav_region.rs` — emits framework's nav + pane-cycle rows when `pane.input_mode() == Navigable`, then pane's `(Nav, _)` rows.
- `pane_action_region.rs` — emits pane's `(PaneAction, _)` rows.
- `global_region.rs` — emits `GlobalAction` + `AppGlobals::render_order()`; suppressed when `pane.input_mode() == TextInput`.

Depends on Phase 10 (`Framework<Ctx>` exists; framework-pane `Shortcuts<Ctx>` impls exist) plus Phase 8's `Keymap<Ctx>` lookups.

Snapshot tests in this phase cover the framework panes only (Settings Browse / Settings Editing / Keymap Browse / Keymap Awaiting / Keymap Conflict / Toasts) plus a fixture pane exercising every `BarRegion` rule. App-pane snapshots land in Phase 12 once their `Shortcuts<App>` impls exist.

**Paired-row separator policy.** Phase 2 tests assert `display_short` never emits `,` or `/` for keys the parser produces (named keys, F1–F12, printable ASCII excluding `,` and `/`). The `Paired` row's `debug_assert!` inherits that contract: bindings constructed from `KeyCode::Media(_)`, `KeyCode::Modifier(_)`, or other exotic variants the parser does not emit are **not covered** and may panic in `Paired` slots. Acceptable trade — the framework owns the `Paired` separator format and only the parser-producible set is bindable from user TOML. If a future binding source widens that set, Phase 2's `display_short_no_separators` test must be widened in lockstep.

### Phase 12 — App action enums + `Shortcuts<App>` impls

**Parallel-path invariant for Phases 12–15.** The new dispatch path lands alongside the old one. The old path stays the source of truth for behavior; the new path is exercised by tests added in each phase. **Phase 16 is the only phase that deletes** old code.

In the cargo-port binary crate:

- Define `NavigationAction`, `FinderAction`, `OutputAction`, `AppGlobalAction`.
- **Split today's `GlobalAction`** in `src/tui/keymap.rs` into `tui_pane::GlobalAction` (the framework half: Quit/Restart/NextPane/PrevPane/OpenKeymap/OpenSettings/Dismiss) and `AppGlobalAction` (binary-owned). During Phases 12–15 the binary's existing `GlobalAction` stays in place; references to the framework's enum are path-qualified as `tui_pane::GlobalAction` to disambiguate. Phase 16 deletes the binary's old enum and `use tui_pane::GlobalAction` makes the name available unqualified.
- Add `ExpandRow` / `CollapseRow` to `ProjectListAction`.
- Implement `Shortcuts<App>` for each app pane (Package, Git, ProjectList, CiRuns, Lints, Targets, Output, Lang, Cpu, Finder). Each pane:
  - Owns `defaults() -> Bindings<Action>`.
  - Owns `shortcut(&self, action, ctx) -> Option<Shortcut>` — moves cursor-position-dependent label logic out of `App::enter_action` into the four affected impls (Package Activate label "open" for `CratesIo`, Git Activate label, Targets Activate label, CiRuns Activate hidden at EOL).
  - Registers a free dispatcher `fn(Action, &mut App)`.
  - Optionally overrides `bar_rows(ctx)` for paired layouts and data-dependent omission (ProjectList: emits `(Nav, Paired(CollapseRow, ExpandRow, "expand"))` and `(Nav, Paired(ExpandAll, CollapseAll, "all"))`; CiRuns: omits toggle row when no ci data).
  - Overrides `input_mode` for `FinderPane` → `TextInput`, `OutputPane` → `Static`. App list panes accept the default `Navigable`.
  - Overrides `vim_extras` to declare pane-action vim binds (`ProjectListAction::ExpandRow → 'l'`, `CollapseRow → 'h'`).
- Implement `Navigation<App> for AppNavigation` and `Globals<App> for AppGlobalAction`.
- Build the app's `Keymap` at startup. Old `App::enter_action` and old `for_status_bar` still exist; the new keymap is populated but not consumed yet.

**`anyhow` lands in the binary in this phase.** This is the first call site that benefits from context wrapping (`Keymap::<App>::builder(...).load_toml(path)?.build()?` → wrap with `.with_context(|| format!("loading keymap from {path:?}"))`). Add `anyhow = "1"` to the root `Cargo.toml` `[dependencies]`. The library (`tui_pane`) does not depend on `anyhow` — only typed `KeymapError` / `KeyParseError` / etc. cross the framework boundary, and the binary adds context at the boundary.

**Phase 12 tests:**
- CiRuns `pane.shortcut(Activate, ctx)` returns `None` when the viewport cursor is at EOL.
- Package `pane.shortcut(Activate, ctx)` returns `Some("open")` when on `CratesIo` field with a version, else `None`.
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

Delete:

- `App::enter_action`, `shortcuts::enter()` const fn.
- The old combined `GlobalAction` enum in `src/tui/keymap.rs` (split into `tui_pane::GlobalAction` + `AppGlobalAction` in Phase 12).
- The seven static constants (`NAV`, `ARROWS_EXPAND`, `ARROWS_TOGGLE`, `TAB_PANE`, `ESC_CANCEL`, `ESC_CLOSE`, `EXPAND_COLLAPSE_ALL`) and all their call sites.
- `detail_groups` / `ci_groups` / `lints_groups` / `project_list_groups` per-context helpers.
- Threaded `enter_action` / `is_rust` / `clear_lint_action` / `terminal_command_configured` / `selected_project_is_deleted` parameters.
- The dead `enter_action` arm in `project_list_groups`.
- The CiRuns `Some("fetch")` label at EOL (the bar bug).
- The bogus `const` on `Shortcut::from_keymap` / `disabled_from_keymap`.

After Phase 16, `shortcuts.rs` contains only legacy types pending removal (or is deleted entirely if all callers have flipped to `Shortcuts::shortcut`). The `InputContext` enum is deleted; tests under `src/tui/app/tests/` referencing it migrate to `app.focus.current()`-based lookups in this phase.

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

- Globals render order matches the framework's render order then `AppGlobals::render_order()`.
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

> Full design in `phase-02-test-infra.md`.

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
- **Framework grants `&mut Vec<Shortcut>` to bar code.** Framework convention: each helper pushes only into vecs it owns content for. Reviewed at PR time.
- **Existing user TOML configs.** New scope names (`[finder]`, `[output]`, `[navigation]`, …) are additive; old configs without these tables still parse and use defaults. No breaking change.

---

## Definition of done

- Workspace exists with `tui_pane` member crate; binary crate consumes it.
- `tui_pane` exposes: `KeyBind`, `Bindings<A>`, `bindings!`, `ScopeMap<A>`, `Keymap<Ctx>` + `KeymapBuilder<Ctx>`, `Shortcuts<Ctx>`, `Navigation<Ctx>`, `Globals<Ctx>`, `Shortcut` + `ShortcutState`, `BarRow<A>`, `GlobalAction`, `VimMode`, `KeymapPane<Ctx>`, `SettingsPane<Ctx>`, `Toasts<Ctx>`, `SettingsRegistry<Ctx>`, `Framework<Ctx>`.
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
