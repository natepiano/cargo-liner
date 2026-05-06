# `tui_pane` — Formal Public API Specification

This file consolidates the public surface of the `tui_pane` crate as it is sketched throughout `docs/tui-pane-lib.md`. Every section below is the canonical signature for the named type or trait. Where the design doc left a choice open, this spec makes the call inline (one sentence) and proceeds.

The crate is generic over a single app context type `Ctx` (the binary supplies `type Ctx = App;`). Every callback the framework invokes while it holds a live `&mut Ctx` borrow is a free function pointer. Settings get/set are closures because the closure itself *is* the borrow holder.

---

## 1. `KeyBind` and friends

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single keystroke: a `KeyCode` plus its modifier flags.
///
/// Values are constructed with `From<KeyCode>` / `From<char>` for the
/// modifier-free common case, the `shift` / `ctrl` / `plain` constructors
/// when modifiers matter, and `from_event` when bridging from crossterm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyBind {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl From<KeyCode> for KeyBind {
    /// `KeyCode::Enter` becomes `KeyBind { code: Enter, mods: NONE }`.
    fn from(code: KeyCode) -> Self {
        Self { code, mods: KeyModifiers::NONE }
    }
}

impl From<char> for KeyBind {
    /// `'c'` becomes `KeyBind { code: Char('c'), mods: NONE }`.
    fn from(c: char) -> Self {
        Self { code: KeyCode::Char(c), mods: KeyModifiers::NONE }
    }
}

impl KeyBind {
    /// Modifier-free bind. Equivalent to `KeyBind::from(code.into())`.
    pub fn plain(code: impl Into<KeyCode>) -> Self {
        Self { code: code.into(), mods: KeyModifiers::NONE }
    }

    /// Shift-modified bind. `KeyBind::shift('g')` → `KeyBind { Char('g'), SHIFT }`.
    pub fn shift(code: impl Into<KeyCode>) -> Self {
        Self { code: code.into(), mods: KeyModifiers::SHIFT }
    }

    /// Control-modified bind. `KeyBind::ctrl('k')` → `KeyBind { Char('k'), CONTROL }`.
    pub fn ctrl(code: impl Into<KeyCode>) -> Self {
        Self { code: code.into(), mods: KeyModifiers::CONTROL }
    }

    /// Build a `KeyBind` from a crossterm `KeyEvent`. Drops fields the
    /// framework does not bind on (`kind`, `state`).
    pub fn from_event(event: KeyEvent) -> Self {
        Self { code: event.code, mods: event.modifiers }
    }

    /// Full display name, e.g. `"Up"`, `"Enter"`, `"Esc"`, `"Ctrl+K"`,
    /// `"Shift+Tab"`. Used by the keymap-overlay help screen.
    pub fn display(&self) -> String { /* … */ unimplemented!() }

    /// Compact display: arrow keys render as glyphs (`↑`, `↓`, `←`, `→`),
    /// every other key delegates to `display`. Used by the status bar.
    /// Must not produce a string containing `,` or `/` for any key the
    /// app uses in a `BarRow::Paired` slot — `bar/` `debug_assert!`s this.
    pub fn display_short(&self) -> String { /* … */ unimplemented!() }

    /// Parse a TOML-style key string (e.g. `"Enter"`, `"Ctrl+K"`,
    /// `"Shift+Tab"`, `"+"`, `"="`). The pre-refactor `+`/`=` collapse
    /// is dropped — `"="` parses to `KeyCode::Char('=')` and `"+"` to
    /// `KeyCode::Char('+')`.
    pub fn parse(s: &str) -> Result<Self, KeyParseError> { /* … */ unimplemented!() }
}

/// Error returned by `KeyBind::parse`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyParseError {
    Empty,
    UnknownKey(String),
    UnknownModifier(String),
    InvalidChar(String),
}

impl std::fmt::Display for KeyParseError { /* … */ }
impl std::error::Error for KeyParseError {}
```

**Tradeoff.** Free constructors (`plain` / `shift` / `ctrl`) over a builder so multi-key literals stay one line. `From<KeyCode>` / `From<char>` keep the `bindings!` macro arms terse.

---

## 2. `Bindings<A>` builder

```rust
/// Default-key declaration for a single scope. Built from a pane's
/// `Shortcuts::defaults` impl (typically via the `bindings!` macro),
/// then folded into the keymap during `KeymapBuilder::build()`.
///
/// Insertion order is significant: the **first** key bound to an action
/// is the action's primary key (what the bar shows when only one key
/// fits). Tests lock this for arrow-vs-vim conflicts.
pub struct Bindings<A> {
    entries: Vec<(KeyBind, A)>,
}

impl<A> Bindings<A> {
    /// Empty builder.
    pub fn new() -> Self { Self { entries: Vec::new() } }

    /// Bind one key to one action. Multiple `bind` calls with the same
    /// action append additional keys; the first call's key is primary.
    pub fn bind(&mut self, key: impl Into<KeyBind>, action: A) -> &mut Self {
        self.entries.push((key.into(), action));
        self
    }

    /// Bind every key in the iterator to the same action, in order.
    /// The first key in the iterator becomes the primary if the action
    /// has no prior binding.
    pub fn bind_many(
        &mut self,
        keys: impl IntoIterator<Item = KeyBind>,
        action: A,
    ) -> &mut Self {
        for k in keys { self.entries.push((k, action.clone())); }
        self
    }

    /// Consume the builder and return the fully indexed `ScopeMap<A>`.
    /// Panics on cross-action collision under `debug_assertions`
    /// (defaults are author-controlled; collisions are bugs, not user
    /// input). User-supplied TOML goes through `load.rs`, which returns
    /// `Result` for the same condition.
    pub fn into_scope_map(self) -> ScopeMap<A>
    where A: Copy + Eq + std::hash::Hash {
        let mut map = ScopeMap::new();
        for (k, a) in self.entries { map.insert(k, a); }
        map
    }
}

impl<A> Default for Bindings<A> {
    fn default() -> Self { Self::new() }
}

impl<A: Clone> Clone for Bindings<A> {
    fn clone(&self) -> Self { Self { entries: self.entries.clone() } }
}
```

`Bindings<A>` is `Default` (free `new()` mirror, idiomatic) and `Clone` when `A: Clone` (lets a pane reuse a defaults table as a starting point for tests). It is **not** `Copy`: the inner `Vec` precludes it, and the framework never needs `Copy` since `into_scope_map` consumes.

### `bindings!` macro

The macro accepts the syntax `KEY => ACTION` (single key) and `[KEY, KEY, …] => ACTION` (multi-key list). Each `KEY` is any `impl Into<KeyBind>` expression — `KeyCode::Enter`, `'c'`, `KeyBind::shift('g')`, `KeyBind::ctrl('k')`. The macro expands to:

```rust
// bindings! { KeyCode::Enter => PackageAction::Activate, ['=', '+'] => ProjectListAction::ExpandAll, … }
// expands to:
{
    let mut __b: Bindings<_> = Bindings::new();
    __b.bind(KeyCode::Enter, PackageAction::Activate);
    __b.bind_many(
        [KeyBind::from('='), KeyBind::from('+')],
        ProjectListAction::ExpandAll,
    );
    // …
    __b
}
```

**Tradeoff.** Macro returns `Bindings<A>` (not `ScopeMap<A>`) so a pane impl can layer extra binds before yielding to the framework. The framework calls `into_scope_map` itself during `build()`.

---

## 3. `ScopeMap<A>`

```rust
use std::collections::HashMap;
use std::hash::Hash;

/// Resolved binding table for a single scope.
///
/// Two indexes:
///   - `by_key`:    1-to-1 within a scope. Used by the dispatcher.
///   - `by_action`: 1-to-many. Insertion order preserved per action;
///                  the first entry in each `Vec<KeyBind>` is the
///                  action's primary key (used by the bar).
///
/// Invariant locked by tests:
///     by_key.len() == by_action.values().map(Vec::len).sum::<usize>()
/// — every key in `by_key` appears exactly once across all `by_action`
/// vectors. No orphans, no double-counts.
pub struct ScopeMap<A: Copy + Eq + Hash> {
    by_key:    HashMap<KeyBind, A>,
    by_action: HashMap<A, Vec<KeyBind>>,
}

impl<A: Copy + Eq + Hash> ScopeMap<A> {
    /// Empty map. `pub(crate)` — only `Bindings::into_scope_map` and the
    /// TOML loader construct one; consumers always receive a built map
    /// from the `Keymap`.
    pub(crate) fn new() -> Self {
        Self { by_key: HashMap::new(), by_action: HashMap::new() }
    }

    /// Insert one (key, action) pair.
    ///
    /// `pub(crate)` — same reason as `new`.
    ///
    /// `debug_assert!`s that `key` is either unbound or already bound
    /// to the same `action`. Cross-action collisions inside one scope
    /// are bugs in `defaults()`; the TOML loader catches the same
    /// condition for user input and returns `Err` instead of panicking.
    pub(crate) fn insert(&mut self, key: KeyBind, action: A) {
        debug_assert!(
            self.by_key.get(&key).is_none_or(|&existing| existing == action),
            "ScopeMap::insert: key {key:?} already maps to a different action",
        );
        if self.by_key.insert(key, action).is_none() {
            self.by_action.entry(action).or_default().push(key);
        }
    }

    /// Dispatcher lookup. `pub` — every input handler calls this.
    pub fn action_for(&self, key: &KeyBind) -> Option<A> {
        self.by_key.get(key).copied()
    }

    /// Primary-key lookup (the first key bound to `action`). `pub` —
    /// the bar reads this when rendering a `BarRow::Paired` slot.
    pub fn key_for(&self, action: A) -> Option<&KeyBind> {
        self.by_action.get(&action).and_then(|v| v.first())
    }

    /// Display the primary key, full name (`"Up"`, `"Ctrl+K"`).
    /// `pub` — keymap-overlay help screen renders these.
    pub fn display_key_for(&self, action: A) -> String {
        self.key_for(action).map(KeyBind::display).unwrap_or_default()
    }

    /// All keys bound to `action`, insertion order. `pub` — the bar's
    /// `Single` row joins these with `,` after `display_short`.
    /// Returns an empty slice for unbound actions.
    pub fn display_keys_for(&self, action: A) -> &[KeyBind] {
        self.by_action.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }
}
```

**Tradeoff.** `new` and `insert` are `pub(crate)` — the framework owns construction. App code only ever receives a built `&ScopeMap<A>` via `Keymap::scope_for`, so a public mutable surface would be a hazard with no caller. `action_for` / `key_for` / `display_key_for` / `display_keys_for` are all `pub` because the bar code (in `tui_pane`) and the input handlers (in the binary) both call them.

---

## 4. `Shortcuts<Ctx>`, `Navigation<Ctx>`, `Globals<Ctx>` traits

```rust
/// Marker trait every action enum implements (typically via the
/// `action_enum!` macro). Provides a static `ALL` slice for
/// declaration-order rendering and the per-variant TOML key + label.
pub trait ActionEnum: Copy + Eq + std::hash::Hash + 'static {
    /// Every variant in declaration order.
    const ALL: &'static [Self];

    /// TOML key for this variant (e.g. `"activate"`).
    fn toml_key(self) -> &'static str;

    /// Human-readable description (used by the keymap-overlay help).
    fn description(self) -> &'static str;
}

/// Pane scope: state-bearing, one impl per pane type.
///
/// `'static` super-trait bound is required because the framework keys
/// the registry on `TypeId<P>` and stores `fn` pointers — both demand
/// `'static`. Pane *instances* live on the binary's `App`; the trait
/// impl itself never holds borrowed data.
pub trait Shortcuts<Ctx>: 'static {
    /// The pane's action enum.
    type Action: ActionEnum;

    /// TOML table name. Survives type renames; one-line cost. Required.
    const SCOPE_NAME: &'static str;

    /// Stable per-instance identity used by the framework's per-pane
    /// query registry (e.g. `input_mode_queries`). One variant per
    /// registered pane; the binary defines this enum.
    fn pane_id() -> PaneId;

    /// Default keybindings. No framework default — every pane declares
    /// its own keys.
    fn defaults() -> Bindings<Self::Action>;

    /// Per-action bar entry: `None` hides the row, `Some(Enabled(label))`
    /// shows it active, `Some(Disabled(label))` shows it greyed out.
    /// Cursor-position-dependent label logic lives here.
    fn shortcut(&self, action: Self::Action, ctx: &Ctx) -> Option<Shortcut>;

    /// Bar row layout. Default: one `(PaneAction, Single(action))` per
    /// `Action::ALL` in declaration order. Override to introduce
    /// `Paired` rows, route into `BarRegion::Nav`, or omit
    /// data-dependent rows.
    fn bar_rows(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarRow<Self::Action>)> {
        Self::Action::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarRow::Single(a)))
            .collect()
    }

    /// Pane's current input mode (Navigable / Static / TextInput).
    /// Drives bar-region suppression and the structural Esc gate.
    fn input_mode(&self, _ctx: &Ctx) -> InputMode { InputMode::Navigable }

    /// Free-fn variant of `input_mode`, registered with the
    /// `Framework<Ctx>::input_mode_queries` map at `register::<P>()`
    /// time. Default forwards to a dummy that returns `Navigable`;
    /// panes whose mode varies with `Ctx` state override.
    fn input_mode_query() -> fn(&Ctx) -> InputMode {
        |_ctx| InputMode::Navigable
    }

    /// Optional vim-extras: pane actions that gain a vim binding when
    /// `VimMode::Enabled`. Default empty. Used by
    /// `ProjectListAction::ExpandRow` (`'l'`) and `CollapseRow` (`'h'`).
    fn vim_extras() -> &'static [(Self::Action, KeyBind)] { &[] }

    /// Free-function dispatcher. Framework calls
    /// `Self::dispatcher()(action, ctx)` while holding `&mut Ctx`.
    /// Implementations navigate from the `Ctx` root rather than
    /// holding a `&mut self` borrow (split-borrow rule).
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}

/// Navigation scope. One impl per app (the binary defines a single
/// `AppNavigation` zero-sized type and impls this trait for it).
///
/// `'static` is implied by the `ActionEnum` bound on `Action`.
pub trait Navigation<Ctx>: 'static {
    type Action: ActionEnum;

    const SCOPE_NAME: &'static str = "navigation";
    const UP:    Self::Action;
    const DOWN:  Self::Action;
    const LEFT:  Self::Action;
    const RIGHT: Self::Action;

    fn defaults() -> Bindings<Self::Action>;

    /// Free fn the framework calls when any navigation action fires.
    /// `focused` lets the dispatcher pick the right scrollable surface.
    fn dispatcher() -> fn(Self::Action, focused: PaneId, ctx: &mut Ctx);
}

/// App-extension globals scope. One impl per app. The framework's own
/// pane-management/lifecycle globals live in `BaseGlobalAction` and are
/// not part of this scope.
pub trait Globals<Ctx>: 'static {
    type Action: ActionEnum;

    const SCOPE_NAME: &'static str = "global";

    fn render_order() -> &'static [Self::Action];
    fn defaults() -> Bindings<Self::Action>;
    fn bar_label(action: Self::Action) -> &'static str;
    fn dispatcher() -> fn(Self::Action, &mut Ctx);
}
```

**Tradeoff.** All three traits are `'static`. The framework stores `fn` pointers and TypeId-keyed lookups; both require `'static`. App pane structs are owned by `App` and are themselves `'static`, so the bound costs nothing in practice.

`Shortcuts::input_mode` exists in two forms — an instance method (`&self`) callers can use directly when they hold the pane, and a free-fn `input_mode_query()` registered with the framework. The free-fn form is what `Framework<Ctx>::input_mode_queries` stores, because the framework only knows the `PaneId` and `&Ctx` at query time. The default `input_mode_query` returns `Navigable`; panes whose mode varies (Finder, Output, Settings) override it explicitly.

---

## 5. `Shortcut` / `ShortcutState` / `BarRow<A>` / `BarRegion` / `InputMode`

```rust
/// Per-action enabled/disabled flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutState { Enabled, Disabled }

/// One bar entry's renderable payload (label + state). The framework
/// adds the bound key on the side via `display_keys_for(action)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub label: &'static str,
    pub state: ShortcutState,
}

impl Shortcut {
    pub fn enabled(label: &'static str) -> Self {
        Self { label, state: ShortcutState::Enabled }
    }

    pub fn disabled(label: &'static str) -> Self {
        Self { label, state: ShortcutState::Disabled }
    }
}

/// One row in the bar. `Single` shows every key bound to one action,
/// joined by `,`. `Paired` glues two actions with `/` under one label,
/// using **primary keys only** — alternative bindings for paired
/// actions never appear in paired slots.
#[derive(Clone, Copy, Debug)]
pub enum BarRow<A> {
    Single(A),
    Paired(A, A, &'static str),
}

/// Three regions of the bar, left-to-right.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarRegion {
    /// Paired-key rows: nav (↑/↓), expand (←/→), all (+/-), pane (Tab).
    Nav,
    /// Per-action rows from the focused pane.
    PaneAction,
    /// `BaseGlobalAction` strip + `Globals::render_order()`.
    Global,
}

impl BarRegion {
    pub const ALL: &'static [BarRegion] =
        &[BarRegion::Nav, BarRegion::PaneAction, BarRegion::Global];
}

/// Pane's current typing/scrolling mode. Drives bar-region suppression
/// and the structural Esc pre-handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    /// List/cursor pane. `NavigationAction` drives it; `Nav` region
    /// emitted; `Global` emitted; structural Esc enabled.
    Navigable,
    /// No scrolling, no typed input (Output, Toasts, KeymapPane
    /// Conflict). `Nav` suppressed; `Global` emitted.
    Static,
    /// Pane consumes typed characters (Finder, Settings Editing,
    /// KeymapPane Awaiting). `Nav` and `Global` suppressed; structural
    /// Esc suppressed.
    TextInput,
}
```

**Tradeoff.** `Shortcut::label` is `&'static str` because every label in cargo-port today is a literal; supporting owned strings would cost an allocation per bar render with no caller asking for it. If a future pane needs a runtime-computed label, lift the field to `Cow<'static, str>` then.

---

## 6. `Keymap<Ctx>`

```rust
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;

/// Built-once, read-every-frame container. TypeId-keyed per scope.
///
/// `PhantomData<fn(&mut Ctx)>` (rather than `PhantomData<Ctx>`) keeps
/// `Keymap<Ctx>` invariant in `Ctx` and *not* auto-`Send`/`Sync` from
/// `Ctx` alone — the actual `Send`/`Sync` impls flow from the
/// `HashMap<TypeId, Box<dyn Any + Send + Sync>>` interior.
pub struct Keymap<Ctx> {
    scopes: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
    base_globals: ScopeMap<BaseGlobalAction>,
    _ctx: PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: 'static> Keymap<Ctx> {
    /// Entry point. Three positional dispatch hooks are required;
    /// omitting any is a compile error rather than a runtime panic.
    pub fn builder(
        quit:    fn(&mut Ctx),
        restart: fn(&mut Ctx),
        dismiss: fn(&mut Ctx),
    ) -> KeymapBuilder<Ctx> { KeymapBuilder::new(quit, restart, dismiss) }

    /// Pane scope lookup. `pub` — input handlers and bar code call this.
    pub fn scope_for<P: Shortcuts<Ctx>>(&self) -> &ScopeMap<P::Action> {
        self.scopes
            .get(&TypeId::of::<P>())
            .and_then(|b| b.downcast_ref::<ScopeMap<P::Action>>())
            .expect("scope_for: pane not registered")
    }

    /// Navigation scope lookup. `pub` — base-pane input handlers call.
    pub fn navigation<N: Navigation<Ctx>>(&self) -> &ScopeMap<N::Action> {
        self.scopes
            .get(&TypeId::of::<N>())
            .and_then(|b| b.downcast_ref::<ScopeMap<N::Action>>())
            .expect("navigation: scope not registered")
    }

    /// App-globals scope lookup. `pub`.
    pub fn globals<G: Globals<Ctx>>(&self) -> &ScopeMap<G::Action> {
        self.scopes
            .get(&TypeId::of::<G>())
            .and_then(|b| b.downcast_ref::<ScopeMap<G::Action>>())
            .expect("globals: scope not registered")
    }

    /// Framework-internal scope. `pub(crate)` — only the framework
    /// dispatcher and the bar's `Global` region read it; the binary
    /// never names `BaseGlobalAction` directly.
    pub(crate) fn base_globals(&self) -> &ScopeMap<BaseGlobalAction> {
        &self.base_globals
    }

    /// Returns `{dirs::config_dir()}/<name>/keymap.toml` for use with
    /// `KeymapBuilder::load_toml`. App supplies its own name (e.g.
    /// `"cargo-port"`). `pub` and free of `Ctx` because it does not
    /// touch the keymap state.
    pub fn config_path(name: &str) -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_default();
        p.push(name);
        p.push("keymap.toml");
        p
    }
}
```

**Tradeoff.** `base_globals` is `pub(crate)` — exposing it would let app code rebind framework-owned actions outside the builder, which the design explicitly rejects (the binary supplies dispatch hooks via `Keymap::builder(quit, restart, dismiss)`, full stop). `config_path` is associated rather than free so callers find it on the type they already mention.

---

## 7. `KeymapBuilder<Ctx>`

```rust
use std::collections::HashMap;
use std::path::Path;

/// Builder for `Keymap<Ctx>`. Construction is a flat builder (no
/// type-state) — the binary calls `register` and friends in any order,
/// and `build()` validates required pieces at runtime by returning
/// `Result<Keymap<Ctx>, BuilderError>`.
pub struct KeymapBuilder<Ctx> {
    // construction state — all private
    quit:    fn(&mut Ctx),
    restart: fn(&mut Ctx),
    dismiss: fn(&mut Ctx),
    vim:     VimMode,
    /// Per-scope `Bindings<A>` boxed via `Any`, keyed by `TypeId<P>` /
    /// `TypeId<N>` / `TypeId<G>`.
    pending: HashMap<TypeId, PendingScope<Ctx>>,
    /// Per-PaneId input-mode queries, captured at `register::<P>()`.
    input_mode_queries: HashMap<PaneId, fn(&Ctx) -> InputMode>,
    settings:    SettingsRegistry<Ctx>,
    framework_accessor: Option<fn(&mut Ctx) -> &mut Framework<Ctx>>,
    nav_registered: bool,
    globals_registered: bool,
    toml_path: Option<PathBuf>,
}

impl<Ctx: 'static> KeymapBuilder<Ctx> {
    /// Toggle vim mode. Optional — defaults to `VimMode::Disabled`.
    pub fn vim_mode(mut self, mode: VimMode) -> Self { self.vim = mode; self }

    /// Register a pane's `Shortcuts<Ctx>` impl. Repeatable. Required
    /// at least once in practice (a keymap with zero pane scopes has
    /// nothing to dispatch), but not enforced — `build()` succeeds.
    pub fn register<P: Shortcuts<Ctx>>(mut self) -> Self { /* … */ self }

    /// Register the navigation scope. **Required.** `build()` returns
    /// `Err(BuilderError::NavigationMissing)` if omitted.
    pub fn with_navigation<N: Navigation<Ctx>>(mut self) -> Self { /* … */ self }

    /// Register the app-globals scope. **Required.** `build()` returns
    /// `Err(BuilderError::GlobalsMissing)` if omitted.
    pub fn with_globals<G: Globals<Ctx>>(mut self) -> Self { /* … */ self }

    /// Populate the settings registry. Optional — apps with no
    /// configurable settings skip it.
    pub fn with_settings(
        mut self,
        f: impl FnOnce(&mut SettingsRegistry<Ctx>),
    ) -> Self {
        f(&mut self.settings);
        self
    }

    /// Register the accessor that takes `&mut Ctx` and yields a
    /// `&mut Framework<Ctx>`. **Required.** Framework panes (Toasts,
    /// SettingsPane, KeymapPane) need this to mutate their own state
    /// from a free-fn dispatcher. `build()` returns
    /// `Err(BuilderError::FrameworkAccessorMissing)` if omitted.
    pub fn with_framework_accessor(
        mut self,
        accessor: fn(&mut Ctx) -> &mut Framework<Ctx>,
    ) -> Self { self.framework_accessor = Some(accessor); self }

    /// Optionally apply user TOML. Returns `Self` (not `Result<Self>`)
    /// because TOML errors are deferred to `build()` — the chain stays
    /// fluent. Errors surface as `BuilderError::Toml(KeymapError)`.
    pub fn load_toml(mut self, path: impl AsRef<Path>) -> Self {
        self.toml_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Validate, fold defaults + TOML + vim, and produce the final
    /// `Keymap<Ctx>`.
    pub fn build(self) -> Result<Keymap<Ctx>, BuilderError> { /* … */ unimplemented!() }
}

/// Errors the builder can surface at `build()` time. All required
/// pieces missing produce a distinct variant — the binary fixes the
/// call site rather than running into a panic.
#[derive(Debug)]
pub enum BuilderError {
    /// `with_navigation::<N>()` was never called.
    NavigationMissing,
    /// `with_globals::<G>()` was never called.
    GlobalsMissing,
    /// `with_framework_accessor(…)` was never called.
    FrameworkAccessorMissing,
    /// TOML file present but unreadable / unparseable / contains
    /// cross-action collisions.
    Toml(KeymapError),
    /// The same pane type was registered twice.
    DuplicateScope { type_name: &'static str },
}

impl std::fmt::Display for BuilderError { /* … */ }
impl std::error::Error for BuilderError {}
```

**Tradeoff.** Flat builder + `Result<Keymap<Ctx>, BuilderError>` from `build()` over a type-state builder. Type-state would catch the three required-method omissions at compile time but at the cost of three extra type parameters on `KeymapBuilder` and a less-readable signature for every intermediate state. The runtime `Result` matches Rust idiom for fallible startup config (TOML errors are fallible regardless), and the binary calls `build()` exactly once at startup so the cost of catching the error there is one `?`.

`load_toml` defers errors to `build()` so the chain reads top-to-bottom without a `?` mid-chain. The framework_accessor requirement is there because the framework panes need to mutate their own state from a free-fn dispatcher (the only place the `&mut Ctx → &mut Framework<Ctx>` projection is known to the framework).

---

## 8. `Framework<Ctx>`

```rust
/// Aggregator owned by the binary's `App`. Holds the three framework
/// panes and the per-PaneId query registry.
///
/// All three pane fields are `pub` so the binary can pass references
/// directly to its bar-render dispatch (`bar::render(&app.framework.toasts, …)`)
/// without going through accessor methods. Mutation goes through each
/// pane's typed methods, not field assignment.
pub struct Framework<Ctx> {
    pub keymap_pane:   KeymapPane<Ctx>,
    pub settings_pane: SettingsPane<Ctx>,
    pub toasts:        Toasts<Ctx>,
    /// Per-PaneId input-mode queries, populated by `KeymapBuilder::register`.
    pub(crate) input_mode_queries: HashMap<PaneId, fn(&Ctx) -> InputMode>,
}

impl<Ctx> Framework<Ctx> {
    /// Construct an empty framework. The binary creates one at startup
    /// and stores it on `App`. `KeymapBuilder::build` later moves the
    /// `input_mode_queries` map into it (via the framework_accessor).
    pub fn new() -> Self {
        Self {
            keymap_pane:   KeymapPane::new(),
            settings_pane: SettingsPane::new(),
            toasts:        Toasts::new(),
            input_mode_queries: HashMap::new(),
        }
    }

    /// Editor-target path for the focused overlay pane (Settings or
    /// Keymap). Replaces today's `overlay_editor_target_path` helper.
    pub fn editor_target_path(&self, focus: PaneId) -> Option<&Path> {
        match focus {
            PaneId::Keymap   => self.keymap_pane.editor_target(),
            PaneId::Settings => self.settings_pane.editor_target(),
            _ => None,
        }
    }

    /// Resolve the focused pane's `InputMode`. Used by the structural
    /// Esc pre-handler and the bar-region suppression logic.
    pub fn focused_pane_input_mode(&self, focus: PaneId, ctx: &Ctx) -> InputMode {
        match focus {
            PaneId::Settings => self.settings_pane.input_mode(),
            PaneId::Keymap   => self.keymap_pane.input_mode(),
            PaneId::Toasts   => InputMode::Static,
            other => self.input_mode_queries
                .get(&other)
                .map_or(InputMode::Navigable, |q| q(ctx)),
        }
    }
}

impl<Ctx> Default for Framework<Ctx> {
    fn default() -> Self { Self::new() }
}
```

**Tradeoff.** Public fields for the three framework panes match the design's intent — the binary uses them directly in `bar::render` dispatch and in input routing. The `input_mode_queries` map is `pub(crate)` because only `KeymapBuilder` (via the framework_accessor) writes it and only `focused_pane_input_mode` reads it.

---

## 9. `SettingsRegistry<Ctx>`

```rust
/// Builder filled inside `KeymapBuilder::with_settings(|registry| …)`.
/// Each setting carries the TOML key, the display label, a getter
/// closure, and a setter closure.
///
/// **Closures, not free fns.** The framework calls these closures while
/// already inside the dispatch path — the closure *is* the borrow holder
/// that touches `Ctx`. There's no re-entrancy hazard, and closures are
/// the more ergonomic choice (they capture `&App` field paths cleanly).
pub struct SettingsRegistry<Ctx> {
    entries: Vec<SettingEntry<Ctx>>,
}

impl<Ctx> SettingsRegistry<Ctx> {
    pub(crate) fn new() -> Self { Self { entries: Vec::new() } }

    /// Bool setting. The setter receives the new value directly.
    pub fn add_bool(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        get:      impl Fn(&Ctx) -> bool + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, bool) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Closed-set enum setting. `variants` is the full list of TOML
    /// keys; the editor-pane cycles through them. Getter returns the
    /// current variant's TOML key; setter receives one of `variants`.
    pub fn add_enum(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        variants: &'static [&'static str],
        get:      impl Fn(&Ctx) -> &'static str + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, &str) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Integer setting with optional min/max. Values outside the range
    /// are rejected by the editor pane before `set` runs.
    pub fn add_int(
        &mut self,
        toml_key: &'static str,
        label:    &'static str,
        get:      impl Fn(&Ctx) -> i64 + Send + Sync + 'static,
        set:      impl Fn(&mut Ctx, i64) + Send + Sync + 'static,
    ) -> &mut Self { /* … */ self }

    /// Set min/max bounds for the most recently added int setting.
    /// Calling on a non-int registers a runtime warning at `build()`.
    pub fn with_bounds(&mut self, min: i64, max: i64) -> &mut Self { /* … */ self }
}

/// Internal record kept per setting.
pub(crate) struct SettingEntry<Ctx> {
    pub toml_key: &'static str,
    pub label:    &'static str,
    pub kind:     SettingKind<Ctx>,
}

pub(crate) enum SettingKind<Ctx> {
    Bool {
        get: Box<dyn Fn(&Ctx) -> bool + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, bool) + Send + Sync>,
    },
    Enum {
        variants: &'static [&'static str],
        get: Box<dyn Fn(&Ctx) -> &'static str + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, &str) + Send + Sync>,
    },
    Int {
        bounds: Option<(i64, i64)>,
        get: Box<dyn Fn(&Ctx) -> i64 + Send + Sync>,
        set: Box<dyn Fn(&mut Ctx, i64) + Send + Sync>,
    },
}
```

**Persistence.** The binary owns persistence. The registry's setter closure is the binary's hook — when the binary writes to its config struct it can also write to disk inside the same closure (or trigger a debounced write later). The framework does **not** open or write any config file on its own; conflating "framework UI" with "binary's config layout" would force every consumer to use one TOML schema. The registry's only persistence-adjacent feature is the `toml_key` field, used by the keymap-overlay help screen and by any export-defaults tooling the binary chooses to add.

---

## 10. `BaseGlobalAction`, `VimMode`, `KeymapError`

```rust
/// Framework-owned global actions. Defaults: q/R/Tab/Shift+Tab/Ctrl+K/s/x.
/// The dispatch for `Quit`, `Restart`, `Dismiss` is supplied by the
/// binary as the three positional `Keymap::builder(quit, restart, dismiss)`
/// arguments. `NextPane`/`PrevPane`/`OpenKeymap`/`OpenSettings` dispatch
/// is owned entirely by the framework (it knows the pane registry).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BaseGlobalAction {
    Quit,
    Restart,
    NextPane,
    PrevPane,
    OpenKeymap,
    OpenSettings,
    Dismiss,
}

impl ActionEnum for BaseGlobalAction {
    const ALL: &'static [Self] = &[
        Self::Quit, Self::Restart, Self::NextPane, Self::PrevPane,
        Self::OpenKeymap, Self::OpenSettings, Self::Dismiss,
    ];

    fn toml_key(self) -> &'static str {
        match self {
            Self::Quit         => "quit",
            Self::Restart      => "restart",
            Self::NextPane     => "next_pane",
            Self::PrevPane     => "prev_pane",
            Self::OpenKeymap   => "open_keymap",
            Self::OpenSettings => "open_settings",
            Self::Dismiss      => "dismiss",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Quit         => "Quit",
            Self::Restart      => "Restart",
            Self::NextPane     => "Next pane",
            Self::PrevPane     => "Previous pane",
            Self::OpenKeymap   => "Open keymap viewer",
            Self::OpenSettings => "Open settings",
            Self::Dismiss      => "Dismiss overlay / output",
        }
    }
}

/// Vim-mode flag passed to `KeymapBuilder::vim_mode`. When `Enabled`,
/// the framework appends `'k'/'j'/'h'/'l'` to `Navigation::UP/DOWN/LEFT/RIGHT`
/// and walks every registered `Shortcuts::vim_extras()` after TOML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimMode {
    Disabled,
    Enabled,
}

impl Default for VimMode {
    fn default() -> Self { Self::Disabled }
}

/// Errors surfaced by the TOML loader. Wrapped by
/// `BuilderError::Toml` when surfaced from `build()`.
#[derive(Debug)]
pub enum KeymapError {
    /// `std::io::Error` opening the file. Missing file is **not** an
    /// error — the loader treats it as "use defaults" and returns `Ok`.
    Io(std::io::Error),
    /// Top-level TOML parse failure.
    Parse(toml::de::Error),
    /// Two TOML keys in the same array refer to the same physical key.
    InArrayDuplicate { scope: String, action: String, key: String },
    /// Two actions in the same scope bind to the same key.
    CrossActionCollision {
        scope: String,
        key: String,
        actions: (String, String),
    },
    /// A TOML key string failed `KeyBind::parse`.
    BadKey { scope: String, action: String, source: KeyParseError },
    /// TOML referenced an unknown action in a known scope.
    UnknownAction { scope: String, action: String },
    /// TOML referenced an unknown scope name.
    UnknownScope { scope: String },
}

impl std::fmt::Display for KeymapError { /* … */ }
impl std::error::Error for KeymapError {}
```

**Tradeoff.** `BaseGlobalAction` impls `ActionEnum` so it can flow through the same `ScopeMap` / TOML / display machinery as every other scope. The binary never names it; the impl exists purely so framework-internal code reuses one path.

---

## 11. `lib.rs` re-exports

```rust
//! `tui_pane` — keymap-driven pane framework on top of crossterm + ratatui.
//!
//! The binary supplies an app context type `Ctx` (typically `App`) and
//! every public type / trait below is generic over it.

#![deny(missing_docs)]

mod bar;
mod framework;
mod keymap;
mod panes;
mod settings;

// --- Keymap core -----------------------------------------------------
pub use crate::keymap::{
    base_globals::BaseGlobalAction,
    bindings::Bindings,
    builder::{BuilderError, KeymapBuilder},
    key_bind::{KeyBind, KeyParseError},
    load::KeymapError,
    mod_::Keymap,
    scope_map::ScopeMap,
    traits::{ActionEnum, Globals, Navigation, Shortcuts},
    vim::VimMode,
};

// --- Bar -------------------------------------------------------------
pub use crate::bar::{
    region::BarRegion,
    shortcut::{BarRow, Shortcut, ShortcutState},
    InputMode,
};

// --- Framework panes + aggregator -----------------------------------
pub use crate::framework::Framework;
pub use crate::panes::{
    keymap_pane::KeymapPane,
    settings_pane::SettingsPane,
    toasts::{Toasts, ToastsAction},
};
pub use crate::settings::SettingsRegistry;

// --- Pane identity ---------------------------------------------------
//
// The framework defines its own three-variant pane id and a wrapper over
// the binary's pane id. `AppContext` is the trait every `Ctx` must impl.
// See `phase-02-paneid-ctx.md` for the full split rationale (option (d)).
pub use crate::app_context::AppContext;
pub use crate::pane_id::{BaseFrameworkPaneId, FocusedPane};

// --- Macros (re-exported at the crate root) -------------------------
pub use crate::keymap::bindings::bindings;
pub use crate::keymap::traits::action_enum;
```

**Final list of exported names** (alphabetical, grouped):

- **Action machinery:** `ActionEnum`, `BaseGlobalAction`, `action_enum!`
- **Keys:** `KeyBind`, `KeyParseError`
- **Bindings & maps:** `Bindings`, `ScopeMap`, `bindings!`
- **Keymap:** `Keymap`, `KeymapBuilder`, `BuilderError`, `KeymapError`, `VimMode`
- **Traits:** `Shortcuts`, `Navigation`, `Globals`
- **Bar:** `BarRegion`, `BarRow`, `Shortcut`, `ShortcutState`, `InputMode`
- **Framework panes & aggregator:** `Framework`, `KeymapPane`, `SettingsPane`, `Toasts`, `ToastsAction`, `SettingsRegistry`
- **Pane identity & context:** `AppContext`, `BaseFrameworkPaneId`, `FocusedPane`

**Tradeoff.** Every `pub use` is from one of five top-level modules (`bar`, `framework`, `keymap`, `panes`, `settings`). The crate root is the only public path; internal module paths are not part of the API. This keeps the binary's `use tui_pane::{KeyBind, Keymap, Shortcuts, …};` flat and stable across internal reorganisations.
